
use engine_native_memory::{Memory, cpdx_array::Array, memory::Raw, read};
use engine_native::{
    ccountry,
    cgame_state,
    crelation::{self, status as crelation_status},
    cwar_relation,
};
use crate::cevent_scope;
use engine_sites::Rva;

#[derive(Debug, Clone, Copy)]
pub struct CallSite {
    pub rva: Rva,
    pub target_rva: Rva,
}

pub const WAR: CallSite = CallSite {
    rva: Rva(0x00d8_b6b8),
    target_rva: Rva(0x00d8_ec50),
};
pub const ANNEX: CallSite = CallSite {
    rva: Rva(0x0071_d4c8),
    target_rva: Rva(0x00ac_af60),
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Declaration,
    Join,
    CivilWarWarTransfer,
    Annex,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CountryIdentity {
    pub reference: u32,
    pub id: u32,
    pub instance: u32,
}
impl CountryIdentity {
    pub fn same_country(self, other: Self) -> bool {
        self.id == other.id
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Operands {
    pub source: Source,
    pub hour: i32,
    pub actor: CountryIdentity,
    pub target: CountryIdentity,
}

#[derive(Debug, Clone, Copy)]
pub struct WarFrame {
    pub rsp_before_call: usize,
    pub rcx: usize,
    pub rdx: usize,
    pub rdi: usize,
}

pub struct WarPending {
    candidate: usize,
    state: usize,
    pair: [u32; 2],
    countries: [usize; 2],
    operands: Operands,
}

impl WarPending {
    pub fn operands(&self) -> Operands {
        self.operands
    }
}

fn at(pointer: usize, offset: usize) -> Result<usize, &'static str> {
    pointer
        .checked_add(offset)
        .ok_or("event feed operand address overflow")
}

fn value<T: Raw>(memory: &impl Memory, pointer: usize, offset: usize) -> Result<T, &'static str> {
    read(memory, at(pointer, offset)?).ok_or("event feed operand unreadable")
}

fn pointer(memory: &impl Memory, object: usize, offset: usize) -> Result<usize, &'static str> {
    usize::try_from(value::<u64>(memory, object, offset)?)
        .map_err(|_| "event feed pointer overflow")
}

fn site(base: usize, key: Rva) -> Result<usize, &'static str> {
    engine_sites::at(base, key).ok_or("event feed operand address overflow")
}

fn verify_vtable(
    memory: &impl Memory,
    base: usize,
    object: usize,
    vtable: Rva,
) -> Result<(), &'static str> {
    if object == 0 || read::<u64>(memory, object) != Some(site(base, vtable)? as u64) {
        return Err("event feed object identity mismatch");
    }
    Ok(())
}

fn state(memory: &impl Memory, base: usize) -> Result<usize, &'static str> {
    let state = pointer(memory, site(base, cgame_state::SINGLETON_RVA)?, 0)?;
    if state == 0 {
        return Err("event feed game state is null");
    }
    Ok(state)
}

pub fn identity(
    memory: &impl Memory,
    base: usize,
    countries: &ccountry::Reader<'_, impl Memory>,
    reference: u32,
) -> Result<(engine_native_memory::country_identity::Country, CountryIdentity), &'static str> {
    let country = if reference == 0 {
        let array = countries.arrays()[0];
        let address = array
            .element(0, 8)
            .ok_or("event feed reserved country absent")?;
        let pointer = pointer(memory, address, 0)?;
        verify_vtable(memory, base, pointer, ccountry::VTABLE_RVA)?;
        if value::<u32>(memory, pointer, ccountry::ID)? != 0 {
            return Err("event feed reserved country ID differs");
        }
        engine_native_memory::country_identity::Country {
            pointer,
            canonical_slot: 0,
            id: 0,
        }
    } else {
        countries.resolve(reference)?
    };
    Ok((
        country,
        CountryIdentity {
            reference,
            id: country.id,
            instance: value(memory, country.pointer, ccountry::SURRENDER_TARGET)?,
        },
    ))
}

fn same(
    countries: &ccountry::Reader<'_, impl Memory>,
    a: u32,
    b: u32,
) -> Result<bool, &'static str> {
    Ok(a == b
        || (a != 0
            && b != 0
            && countries.resolve(a)?.canonical_slot == countries.resolve(b)?.canonical_slot))
}

fn diplomacy(
    memory: &impl Memory,
    base: usize,
    country: engine_native_memory::country_identity::Country,
) -> Result<usize, &'static str> {
    let diplomacy = pointer(memory, country.pointer, ccountry::DIPLOMACY_STATUS)?;
    verify_vtable(memory, base, diplomacy, engine_native_parts::cdiplomacy_status_layout::VTABLE_RVA)?;
    if pointer(memory, diplomacy, engine_native_parts::cdiplomacy_status_layout::COUNTRY)? != country.pointer {
        return Err("event feed diplomacy owner mismatch");
    }
    Ok(diplomacy)
}

fn status(
    memory: &impl Memory,
    base: usize,
    countries: &ccountry::Reader<'_, impl Memory>,
    owner: engine_native_memory::country_identity::Country,
    target: engine_native_memory::country_identity::Country,
    target_reference: u32,
) -> Result<usize, &'static str> {
    let diplomacy = diplomacy(memory, base, owner)?;
    let matrix = Array::read(memory, at(diplomacy, engine_native_parts::cdiplomacy_status_layout::RELATIONS)?, 8)
        .ok_or("event feed relation matrix unreadable or invalid")?;
    let slot = matrix
        .element(target.canonical_slot, 8)
        .ok_or("event feed target outside relation matrix")?;
    let status = pointer(memory, slot, 0)?;
    verify_vtable(memory, base, status, crelation_status::VTABLE_RVA)?;
    if pointer(memory, status, crelation_status::DIPLOMACY_STATUS)? != diplomacy {
        return Err("event feed relation status owner mismatch");
    }
    if !same(
        countries,
        value(memory, status, crelation_status::TARGET_COUNTRY)?,
        target_reference,
    )? {
        return Err("event feed relation status target mismatch");
    }
    Ok(status)
}

fn candidate(
    memory: &impl Memory,
    base: usize,
    candidate: usize,
) -> Result<[u32; 2], &'static str> {
    verify_vtable(memory, base, candidate, cwar_relation::VTABLE_RVA)?;
    if value::<u32>(memory, candidate, crelation::TYPE_ID)? != cwar_relation::TYPE {
        return Err("event feed candidate type mismatch");
    }
    if value::<u8>(memory, candidate, crelation::CANCEL)? != 0
        || value::<u8>(memory, candidate, crelation::RETIRED)? != 0
    {
        return Err("event feed candidate canceled or retired");
    }
    let pair: [u8; 8] = value(memory, candidate, crelation::FIRST_COUNTRY)?;
    Ok([
        u32::from_le_bytes(pair[..4].try_into().unwrap()),
        u32::from_le_bytes(pair[4..].try_into().unwrap()),
    ])
}

const DECLARATION: Rva = Rva(0x00d9_8a1e);
const JOIN: Rva = Rva(0x00d9_ed0a);
const TRANSFER: Rva = Rva(0x0116_8728);
const WAR_FRAME_RETURNS: [Rva; 2] = [Rva(0x00d8_1bff), Rva(0x00d8_167d)];

pub fn war_source_sites(base: usize) -> Result<(), &'static str> {
    engine_sites::require(base, &WAR_FRAME_RETURNS)?;
    engine_sites::require(base, &[DECLARATION, JOIN, TRANSFER])
}

fn war_source(memory: &impl Memory, base: usize, frame: WarFrame) -> Result<Source, &'static str> {
    if value::<u64>(memory, frame.rsp_before_call, 0x1d8)? != site(base, WAR_FRAME_RETURNS[0])? as u64
        || value::<u64>(memory, frame.rsp_before_call, 0x448)?
            != site(base, WAR_FRAME_RETURNS[1])? as u64
        || pointer(memory, frame.rsp_before_call, 0x438)? != frame.rcx
    {
        return Err("event feed war creator frame mismatch");
    }
    let source_pc = pointer(memory, frame.rsp_before_call, 0x4a8)?;
    match engine_sites::which(base, source_pc, &[DECLARATION, JOIN, TRANSFER]) {
        Some(DECLARATION) => Ok(Source::Declaration),
        Some(JOIN) => Ok(Source::Join),
        Some(TRANSFER) => Ok(Source::CivilWarWarTransfer),
        _ => Err("event feed war producer is unclassified"),
    }
}

pub fn war_before(
    memory: &impl Memory,
    base: usize,
    frame: WarFrame,
) -> Result<WarPending, &'static str> {
    let source = war_source(memory, base, frame)?;
    if frame.rdx == 0 || frame.rdx != frame.rdi {
        return Err("event feed candidate register mismatch");
    }
    let pair = candidate(memory, base, frame.rdx)?;
    verify_vtable(memory, base, frame.rcx, crelation_status::VTABLE_RVA)?;
    let owner_diplomacy = pointer(memory, frame.rcx, crelation_status::DIPLOMACY_STATUS)?;
    verify_vtable(memory, base, owner_diplomacy, engine_native_parts::cdiplomacy_status_layout::VTABLE_RVA)?;
    let owner = pointer(memory, owner_diplomacy, engine_native_parts::cdiplomacy_status_layout::COUNTRY)?;
    let actor_reference = value(memory, owner, ccountry::ID)?;
    let target_reference = value(memory, frame.rcx, crelation_status::TARGET_COUNTRY)?;
    let state = state(memory, base)?;
    let countries = ccountry::Reader::new(memory, base, state)?;
    let (actor, actor_identity) = identity(memory, base, &countries, actor_reference)?;
    let (target, target_identity) = identity(memory, base, &countries, target_reference)?;
    if actor.pointer != owner
        || !same(&countries, pair[0], actor_reference)?
        || !same(&countries, pair[1], target_reference)?
    {
        return Err("event feed creator and candidate pair mismatch");
    }
    let selected = status(memory, base, &countries, actor, target, target_reference)?;
    if selected != frame.rcx || diplomacy(memory, base, actor)? != owner_diplomacy {
        return Err("event feed creator status is not selected matrix slot");
    }
    if pointer(memory, selected, crelation_status::WAR_RELATION)? == frame.rdx {
        return Err("event feed candidate already installed before second link");
    }
    Ok(WarPending {
        candidate: frame.rdx,
        state,
        pair,
        countries: [actor.pointer, target.pointer],
        operands: Operands {
            source,
            hour: value(memory, state, cgame_state::CURRENT_HOUR)?,
            actor: actor_identity,
            target: target_identity,
        },
    })
}

pub fn war_returned(
    memory: &impl Memory,
    base: usize,
    pending: WarPending,
) -> Result<Operands, &'static str> {
    let state = state(memory, base)?;
    if state != pending.state {
        return Err("event feed game state changed during native operation");
    }
    let countries = ccountry::Reader::new(memory, base, state)?;
    let (actor, actor_identity) =
        identity(memory, base, &countries, pending.operands.actor.reference)?;
    let (target, target_identity) =
        identity(memory, base, &countries, pending.operands.target.reference)?;
    if !actor_identity.same_country(pending.operands.actor)
        || !target_identity.same_country(pending.operands.target)
        || [actor.pointer, target.pointer] != pending.countries
    {
        return Err("event feed country identity changed during native operation");
    }
    if candidate(memory, base, pending.candidate)? != pending.pair {
        return Err("event feed candidate raw pair changed during native operation");
    }
    let selected = status(
        memory,
        base,
        &countries,
        actor,
        target,
        pending.operands.target.reference,
    )?;
    let reciprocal = status(
        memory,
        base,
        &countries,
        target,
        actor,
        pending.operands.actor.reference,
    )?;
    if pointer(memory, selected, crelation_status::WAR_RELATION)? != pending.candidate
        || pointer(memory, reciprocal, crelation_status::WAR_RELATION)? != pending.candidate
    {
        return Err("event feed candidate reciprocal attachment not proved");
    }
    Ok(pending.operands)
}

#[derive(Debug, Clone, Copy)]
pub struct AnnexFrame {
    pub rbp: usize,
    pub r8: usize,
    pub r9: u64,
}

pub fn annex_before(
    memory: &impl Memory,
    base: usize,
    frame: AnnexFrame,
) -> Result<Operands, &'static str> {
    if frame.r8 != at(frame.rbp, 0xa0)? || frame.r9 != 0 {
        return Err("event feed annex register frame mismatch");
    }
    verify_vtable(memory, base, frame.r8, cevent_scope::VTABLE_RVA)?;
    let from = pointer(memory, frame.r8, cevent_scope::FROM)?;
    verify_vtable(memory, base, from, cevent_scope::VTABLE_RVA)?;
    let state = state(memory, base)?;
    let countries = ccountry::Reader::new(memory, base, state)?;
    let (_, actor) = identity(
        memory,
        base,
        &countries,
        value(memory, frame.r8, cevent_scope::COUNTRY_TAG)?,
    )?;
    let (_, target) = identity(
        memory,
        base,
        &countries,
        value(memory, from, cevent_scope::COUNTRY_TAG)?,
    )?;
    Ok(Operands {
        source: Source::Annex,
        hour: value(memory, state, cgame_state::CURRENT_HOUR)?,
        actor,
        target,
    })
}

pub mod provenance;

#[cfg(test)]
mod tests;
