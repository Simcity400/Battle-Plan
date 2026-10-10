//! Optional strike-readiness operands. Four ship bases are stored simulation
//! words; torpedo cooldown alone needs its effective modifier with the task
//! force's selected fleet leader. No combat side, game call, modifier tree,
//! client state or carried packet supplies these operands.

use crate::{
    Memory, allocation, cnaval_combatant::field, cpdx_array::Array,
    creference_registry::ScopedResolver, modifier_definition,
    naval_ship_modifier_inputs as sources,
};

const ONE: i64 = 100_000;
// Torpedo-cooldown modifier key; root owns its named fixed-layout guard.
const COOLDOWN_KEY: u32 = 0x1be;
pub const METADATA_CAPACITY: &str = "ship strike modifier metadata capacity exceeded";

/// Prepared separately from OrganisationMemo. Only preparation/growth allocates;
/// each reached key retains its complete result, including a failure, until the
/// next acquisition pass. The original organisation path never consults it.
pub struct TorpedoCooldownMemo {
    worker_growth: bool,
    base: usize,
    definitions: Vec<(u32, Result<modifier_definition::Operands, &'static str>)>,
    prepared_definition_count: usize,
    required_definitions: usize,
    registry: ScopedResolver,
}

impl TorpedoCooldownMemo {
    /// Allocation-free placeholder. A reached present key refuses until prepared.
    pub fn vacant(base: usize) -> Self {
        Self {
            worker_growth: false,
            base,
            definitions: Vec::new(),
            prepared_definition_count: 0,
            required_definitions: 0,
            registry: ScopedResolver::new(base),
        }
    }

    /// Off-thread preparation from the validated live modifier-definition header.
    /// Every possible successful key has one row, so this bounds the key memo
    /// without a vanilla count or a second modifier-source census. An OOM remains
    /// the shared allocator's terminal error; an unreadable header is InvalidData.
    pub fn try_prepare(memory: &impl Memory, base: usize) -> std::io::Result<Self> {
        let count = Self::definition_count(memory, base)?;
        Self::from_count(base, count)
    }

    fn from_count(base: usize, count: usize) -> std::io::Result<Self> {
        Ok(Self {
            definitions: allocation::vec(count)?,
            prepared_definition_count: count,
            ..Self::vacant(base)
        })
    }

    /// Off-thread storage preparation from the validated modifier table count
    /// observed for this attachment. The count is only a storage bound; no
    /// metadata values or native pointers survive preparation. The capture owner
    /// must bind its authenticated module base with begin_capture before reading.
    pub fn try_from_validated_count(count: usize) -> std::io::Result<Self> {
        Self::from_count(0, count)
    }

    /// Only while this capture is idle, after every detail part returned. The
    /// caller supplies the monotonically raised count from the same attached
    /// module's validated native header. No reached-key guess sizes this storage.
    pub fn try_grow_from_validated_count(&mut self, count: usize) -> std::io::Result<()> {
        if count > self.definitions.capacity() {
            *self = Self::from_count(self.base, count)?;
        } else {
            self.prepared_definition_count = self.prepared_definition_count.max(count);
            self.clear();
        }
        Ok(())
    }

    /// Once per current part acquisition, before any detail read. Reset every
    /// result and reference observation even when pointers and counts match.
    /// This is allocation-free and does not revalidate or record the sizing hint.
    pub fn begin_capture(&mut self, base: usize) {
        self.base = base;
        self.clear();
    }

    pub fn enable_worker_growth(&mut self) {
        self.worker_growth = true;
    }

    /// Called only after all acquisition jobs have returned. Native metadata can
    /// grow during content loading; the next pass either fits or refuses the
    /// newly reached key without any callback allocation or source retry.
    pub fn try_grow(&mut self, memory: &impl Memory, base: usize) -> std::io::Result<()> {
        let count = Self::definition_count(memory, base)?;
        if self.base != base || count > self.definitions.capacity() {
            *self = Self::from_count(base, count)?;
        } else {
            self.prepared_definition_count = count;
            self.clear();
        }
        Ok(())
    }

    fn definition_count(memory: &impl Memory, base: usize) -> std::io::Result<usize> {
        modifier_definition::table(memory, base)
            .map(|table| table.count as usize)
            .map_err(|_| std::io::ErrorKind::InvalidData.into())
    }

    /// Reset at each hourly acquisition; same addresses do not prove freshness.
    pub fn clear(&mut self) {
        self.definitions.clear();
        self.required_definitions = 0;
        self.registry = ScopedResolver::new(self.base);
    }

    pub fn required_definition_capacity(&self) -> usize {
        self.required_definitions
    }

    pub fn allocated_bytes(&self) -> usize {
        self.definitions.capacity()
            * size_of::<(u32, Result<modifier_definition::Operands, &'static str>)>()
    }

    fn effective(
        &mut self,
        memory: &impl Memory,
        base: usize,
        key: u32,
        raw: i64,
    ) -> Result<i64, &'static str> {
        if base != self.base {
            return Err("ship strike metadata module differs from preparation");
        }
        if let Some((_, result)) = self.definitions.iter().find(|(stored, _)| *stored == key) {
            return (*result).map(|operands| operands.effective(raw));
        }
        self.required_definitions = self
            .required_definitions
            .max(self.definitions.len().saturating_add(1));
        if !self.worker_growth
            && (key as usize >= self.prepared_definition_count
                || self.definitions.len() == self.definitions.capacity())
        {
            return Err(METADATA_CAPACITY);
        }
        // The prepared count admits storage only. A reached present key still
        // authenticates the current native header before reading its operands.
        // Keep either result sticky within this acquisition, including a torn
        // or unreadable header. Original organisation metadata is untouched.
        let table = modifier_definition::table(memory, base);
        if self.worker_growth {
            // The complete definition count in the original metadata header admits
            // every current definition before recording this reached result.
            let count = table.as_ref().map_or(0, |table| table.count as usize);
            let required = count.max(
                self.definitions
                    .len()
                    .checked_add(1)
                    .ok_or("ship strike metadata count overflow")?,
            );
            let additional = required - self.definitions.len();
            allocation::reserve(&mut self.definitions, additional)
                .map_err(|_| "ship strike metadata allocation failed")?;
            self.prepared_definition_count = self.prepared_definition_count.max(count);
            self.required_definitions = self.required_definitions.max(count);
        }
        let result = table.and_then(|table| {
            if key as usize >= table.count as usize {
                Err("ship strike modifier metadata key outside current table")
            } else {
                modifier_definition::operands(memory, base, key)
            }
        });
        // Capacity was prepared above. Retain an error as well as success so a
        // later ship cannot hide this pass's first observation of the same key.
        self.definitions.push((key, result));
        result.map(|operands| operands.effective(raw))
    }
}

/// Current simulation bases and independently acquired cooldown modifier.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ShipStrikeInputs {
    pub light_attack: Option<i64>,
    pub heavy_attack: Option<i64>,
    pub light_piercing: Option<i64>,
    pub torpedo_attack: Option<i64>,
    pub torpedo_cooldown_modifier: Option<i64>,
}

/// Completed original maximum plus an independently refused optional operand.
/// No native pointer or companion escapes the combined acquisition call.
pub struct OrganisationAndCooldown {
    pub maximum_organisation: i64,
    pub torpedo_cooldown_modifier: Result<i64, &'static str>,
}

/// Run the unchanged four-key maximum first, then the isolated optional lane.
/// `suffix_memory` must have its own read budget, read-only access to completed
/// core snapshots, and a separately prepared arena for all newly reached raw
/// successes/failures. Core acquisition must never consult suffix observations.
pub fn read_maximum_organisation_and_torpedo_cooldown(
    memory: &impl Memory,
    suffix_memory: &impl Memory,
    base: usize,
    ship: usize,
    organisation: &mut super::OrganisationMemo,
    cooldown: &mut TorpedoCooldownMemo,
) -> Result<OrganisationAndCooldown, &'static str> {
    let observed = super::read_organisation_observation(memory, base, ship, organisation)?;
    let modifier = read_cooldown(suffix_memory, base, ship, &observed, cooldown);
    Ok(OrganisationAndCooldown {
        maximum_organisation: observed.maximum,
        torpedo_cooldown_modifier: modifier,
    })
}

/// Four independent exact 8-byte spans in summary lane order. The caller
/// already authenticated this ShipModels seed. Missing spans neither stop
/// nor manufacture sibling values.
pub fn read_strike_bases(memory: &impl Memory, ship: usize) -> [Option<i64>; 4] {
    [0x1e8, 0x1f8, 0x210, 0x208].map(|offset| field(memory, ship, offset).ok())
}

fn add(pointer: usize, offset: usize) -> Result<usize, &'static str> {
    pointer
        .checked_add(offset)
        .filter(|_| pointer != 0)
        .ok_or("ship strike pointer overflow")
}

fn ptr(memory: &impl Memory, pointer: usize, offset: usize) -> Result<usize, &'static str> {
    field::<u64>(memory, pointer, offset).map(|value| value as usize)
}

fn auth(memory: &impl Memory, base: usize, pointer: usize, rva: usize) -> Result<(), &'static str> {
    if ptr(memory, pointer, 0)? == add(base, rva)? {
        Ok(())
    } else {
        Err("ship strike source identity mismatch")
    }
}

fn scalar(
    memory: &impl Memory,
    base: usize,
    address: usize,
    key: u32,
    memo: &mut TorpedoCooldownMemo,
) -> Result<i64, &'static str> {
    // lookup_at owns the one optional raw-snapshot probe and its sticky-error
    // versus direct-fallback contract. Do not probe that capability twice.
    let value = crate::modifier_accumulator::lookup_at(memory, address, key)?;
    if value.present {
        memo.effective(memory, base, key, value.raw)
    } else {
        // Native absence skips feature/clamp metadata, while present zero does not.
        Ok(0)
    }
}

fn read_cooldown(
    memory: &impl Memory,
    base: usize,
    ship: usize,
    observed: &super::OrganisationObservation,
    memo: &mut TorpedoCooldownMemo,
) -> Result<i64, &'static str> {
    use crate::{ccountry, cship, ctask_force, cunit};
    if memo.base != base {
        return Err("ship strike metadata module differs from preparation");
    }
    auth(memory, base, ship, cship::VTABLE_RVA)?;
    if field::<u64>(memory, ship, cship::HANDLE)? != observed.handle {
        return Err("ship strike identity differs from organisation invocation");
    }
    let task = ptr(memory, ship, cship::TASK_FORCE)?;
    if task != observed.task {
        return Err("ship strike task differs from organisation invocation");
    }
    auth(memory, base, task, ctask_force::VTABLE_RVA)?;
    // Authenticate the task force's selected fleet leader independently before
    // reusing a result obtained with the original query's default leader.
    // A mismatch never substitutes a leader.
    let fleet = ptr(memory, task, ctask_force::FLEET)?;
    let leader = if fleet == 0 {
        None
    } else {
        auth(memory, base, fleet, crate::cfleet::VTABLE_RVA)?;
        let reference = field::<u64>(memory, fleet, 0xb0)?;
        let leader = memo.registry.resolve(memory, reference)?;
        if let Some(leader) = leader {
            auth(memory, base, leader, crate::cnavy_leader::VTABLE_RVA)?;
            if field::<u64>(memory, leader, crate::cunit_leader::TYPE)? != reference {
                return Err("ship strike leader identity differs from fleet reference");
            }
        }
        leader
    };
    if fleet != observed.fleet || leader != observed.leader {
        return Err("ship strike leader differs from organisation invocation");
    }
    let game = ptr(memory, base, crate::cgame_state::SINGLETON_RVA)?;
    let country_id = field(memory, task, cunit::ROSTER_OWNER)?;
    let country = ccountry::Reader::new(memory, base, game)?
        .resolve(country_id)?
        .pointer;
    let mut total = scalar(memory, base, add(country, 0x5b8)?, COOLDOWN_KEY, memo)?;
    if field::<u64>(memory, country, 0x250)? == observed.handle {
        let root = ptr(memory, base, 0x0331_7cc8)?;
        total = total.wrapping_add(scalar(
            memory,
            base,
            add(ptr(memory, root, 0xb0)?, 0x10)?,
            COOLDOWN_KEY,
            memo,
        )?);
    }
    total = total.wrapping_add(scalar(memory, base, add(ship, 0x750)?, COOLDOWN_KEY, memo)?);
    let medals = ptr(memory, ship, 0x8f8)?;
    total = total.wrapping_add(scalar(
        memory,
        base,
        add(medals, 0x1b0)?,
        COOLDOWN_KEY,
        memo,
    )?);
    total = total.wrapping_add(doctrine(memory, base, task, country, memo)?);
    let location = ptr(memory, task, cunit::LOCATION)?;
    if location != 0 {
        auth(memory, base, location, crate::cprovince::VTABLE_RVA)?;
        let definition = ptr(memory, location, 0xc8)?;
        let terrain = sources::terrain(memory, base, definition)?;
        let class = field(memory, ship, cship::CLASS_DATABASE_ID)?;
        let (modifier, _) = sources::terrain_class(memory, terrain, class)?;
        total = total.wrapping_add(scalar(
            memory,
            base,
            add(modifier, 0x10)?,
            COOLDOWN_KEY,
            memo,
        )?);
        let weather = ptr(memory, game, crate::cgame_state::WEATHER_MANAGER)?;
        auth(memory, base, weather, crate::cweather_manager::VTABLE_RVA)?;
        let regions = Array::read(memory, add(weather, 0x40)?, 0x160)
            .ok_or("ship strike weather region array invalid")?;
        let region = regions
            .element(field(memory, definition, 0x58)?, 0x160)
            .ok_or("ship strike weather region missing")?;
        let value = scalar(memory, base, add(region, 0x58)?, COOLDOWN_KEY, memo)?;
        let country_adjustment = scalar(
            memory,
            base,
            add(country, 0x5b8)?,
            crate::modifier_keys::NAVY_WEATHER_PENALTY,
            memo,
        )?
        .wrapping_add(ONE)
        .clamp(0, ONE);
        let weather_scale = field::<i64>(memory, ship, 0x270)?
            .wrapping_sub(ONE)
            .wrapping_add(country_adjustment);
        total = total.wrapping_add(value.wrapping_mul(weather_scale) / ONE);
        let provinces = Array::read(memory, add(weather, 0x370)?, 0x330)
            .ok_or("ship strike weather province array invalid")?;
        let province = provinces
            .element(field(memory, location, crate::cprovince::ID)?, 0x330)
            .ok_or("ship strike weather province missing")?;
        total = total.wrapping_add(scalar(
            memory,
            base,
            add(province, 0x280)?,
            COOLDOWN_KEY,
            memo,
        )?);
        total = total.wrapping_add(scalar(memory, base, add(task, 0x668)?, COOLDOWN_KEY, memo)?);
    }
    if let Some(leader) = leader {
        total = total.wrapping_add(scalar(
            memory,
            base,
            add(leader, 0x418)?,
            COOLDOWN_KEY,
            memo,
        )?);
        let subunit = field(memory, ship, 0x60c)?;
        let class = sources::selected_class(memory, leader, subunit, true)?;
        total = total.wrapping_add(scalar(memory, base, add(class, 0x10)?, COOLDOWN_KEY, memo)?);
    }
    for index in 0..observed.criticals.count {
        let entry = observed
            .criticals
            .element(index, 16)
            .ok_or("ship strike critical address overflow")?;
        let definition = ptr(memory, entry, 0)?;
        let value = scalar(memory, base, add(definition, 0x98)?, COOLDOWN_KEY, memo)?;
        let count = i64::from(field::<i32>(memory, entry, 8)?);
        // Native truncates each entry twice; the count*ONE step must not be
        // cancelled algebraically, since both multiplications can wrap.
        total = total.wrapping_add(
            (value.wrapping_mul(count).wrapping_mul(ONE) / ONE)
                .wrapping_mul(observed.critical_scale)
                / ONE,
        );
    }
    let influences =
        Array::read(memory, add(task, 0x728)?, 8).ok_or("ship strike support array invalid")?;
    let mut best = 0_i64;
    for index in 0..influences.count {
        let wrapper = ptr(
            memory,
            influences
                .element(index, 8)
                .ok_or("ship strike support address overflow")?,
            0,
        )?;
        let influence = ptr(memory, wrapper, 0x40)?;
        if influence == 0 {
            continue;
        }
        let Some(character) = memo
            .registry
            .resolve(memory, field(memory, influence, 8)?)?
        else {
            continue;
        };
        auth(memory, base, character, crate::ccharacter::VTABLE_RVA)?;
        let direct = ptr(memory, character, crate::ccharacter::UNIT_LEADER)?;
        let leader = if direct == 0 {
            memo.registry
                .resolve(memory, field(memory, character, 0xc8)?)?
        } else {
            Some(direct)
        };
        let Some(leader) = leader else {
            continue;
        };
        if field::<i32>(memory, leader, crate::cunit_leader::CATEGORY)? != 2 {
            continue;
        }
        auth(memory, base, leader, crate::cnavy_leader::VTABLE_RVA)?;
        let mut contribution = 0_i64;
        for offset in [0xf70, 0xf80, 0xfa0, 0xf90] {
            let definition = ptr(memory, leader, offset)?;
            contribution = contribution.wrapping_add(scalar(
                memory,
                base,
                add(definition, 0xe0)?,
                COOLDOWN_KEY,
                memo,
            )?);
        }
        best = best.max(contribution);
    }
    Ok(total.wrapping_add(best))
}

fn doctrine(
    memory: &impl Memory,
    base: usize,
    task: usize,
    country: usize,
    memo: &mut TorpedoCooldownMemo,
) -> Result<i64, &'static str> {
    let mut tree = crate::msvc_tree_node::Cursor::read(memory, task, 0x640)
        .map_err(|_| "ship strike doctrine header invalid")?;
    let mut previous = None;
    let mut total = 0_i64;
    while let Some(node) = tree
        .next(memory)
        .map_err(|_| "ship strike doctrine links or count invalid")?
    {
        let selector = field::<i32>(memory, node, 0x20)?;
        if previous.is_some_and(|value| value >= selector) {
            return Err("ship strike doctrine order invalid");
        }
        previous = Some(selector);
        let modifier = ptr(memory, node, 0x30)?;
        let raw = scalar(memory, base, add(modifier, 0x10)?, COOLDOWN_KEY, memo)?;
        if raw == 0 {
            continue;
        }
        let keys = sources::doctrine_keys::<true>(memory, base, selector as u32)?;
        let numerator = scalar(memory, base, add(country, 0x5b8)?, keys[0], memo)?
            .wrapping_add(ONE)
            .wrapping_mul(i64::from(field::<i32>(memory, node, 0x2c)?))
            .wrapping_mul(ONE)
            / ONE;
        let denominator = i64::from(field::<i32>(memory, node, 0x28)?).wrapping_mul(ONE);
        let ratio = if denominator == 0
            || (numerator as u64).wrapping_add(0x7fff_ffff_ffff_fffe) >= 0xffff_ffff_ffff_fffd
        {
            0
        } else {
            numerator
                .wrapping_mul(ONE)
                .checked_div(denominator)
                .ok_or("ship strike doctrine division overflow")?
                .clamp(0, ONE)
        };
        let factor = scalar(memory, base, add(country, 0x5b8)?, keys[1], memo)?.wrapping_add(ONE);
        total = total.wrapping_add((raw.wrapping_mul(factor) / ONE).wrapping_mul(ratio) / ONE);
    }
    Ok(total)
}
