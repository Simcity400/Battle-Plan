//! Guarded operands for reciprocal war-link and annex dispatch boundaries.
//! Fresh reciprocal matrix checks authenticate attachment. The caller establishes
//! world and thread lifetime independently; these reads establish neither
//! load/replacement eligibility nor safe hook publication.

use crate::{
    Memory, ccountry, cdiplomacy_status, cevent_scope, cgame_state, cpdx_array::Array, crelation,
    crelation_status, cwar_relation, memory::Raw, read,
};
use engine_sites::Rva;

#[derive(Debug, Clone, Copy)]
pub struct CallSite {
    pub rva: Rva,
    pub target_rva: Rva,
}

/// Second war-link CALL: RCX status, RDX candidate retained in RDI.
/// The continuation initializes the war bookkeeping discriminators.
pub const WAR: CallSite = CallSite {
    rva: Rva(0x00d8_b6b8),
    target_rva: Rva(0x00d8_ec50),
};
/// Annex dispatch CALL: R8 the primary scope's frame slot, R9D zero.
pub const ANNEX: CallSite = CallSite {
    rva: Rva(0x0071_d4c8),
    target_rva: Rva(0x00ac_af60),
};

/// Source meanings, not replay discriminant allocations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Declaration,
    Join,
    CivilWarWarTransfer,
    Annex,
}

/// Preserve the source reference independently of its resolved full native CCountry ID and its
/// surrender target; the latter is data, not an incarnation counter.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CountryIdentity {
    pub reference: u32,
    pub id: u32,
    /// Raw surrender target retained under the recording's legacy field name.
    pub instance: u32,
}
impl CountryIdentity {
    pub fn same_country(self, other: Self) -> bool {
        self.id == other.id
    }
}

/// Only owned integer operands; no native address is publishable history.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Operands {
    pub source: Source,
    pub hour: i32,
    pub actor: CountryIdentity,
    pub target: CountryIdentity,
}

/// Register snapshot immediately before the war-link CALL. This RSP differs
/// from Rust callback RSP and the eight-byte-lower stub-entry RSP.
#[derive(Debug, Clone, Copy)]
pub struct WarFrame {
    pub rsp_before_call: usize,
    pub rcx: usize,
    pub rdx: usize,
    pub rdi: usize,
}

/// Opaque correlation for this one original invocation only. It is not Clone,
/// serializable or an event key. The owner retains it on uncertain completion;
/// only a proved normal return may consume it through war_returned.
pub struct WarPending {
    candidate: usize,
    state: usize,
    pair: [u32; 2],
    countries: [usize; 2],
    operands: Operands,
}

impl WarPending {
    /// Owned pre-call operands; attachment needs the normal-return check.
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

/// The country a raw reference resolves to with its full native ID.
pub fn identity(
    memory: &impl Memory,
    base: usize,
    countries: &ccountry::Reader<'_, impl Memory>,
    reference: u32,
) -> Result<(ccountry::Country, CountryIdentity), &'static str> {
    let country = if reference == 0 {
        // Country resolution selects the physical reserved slot zero.
        let array = countries.arrays()[0];
        let address = array
            .element(0, 8)
            .ok_or("event feed reserved country absent")?;
        let pointer = pointer(memory, address, 0)?;
        verify_vtable(memory, base, pointer, ccountry::VTABLE_RVA)?;
        if value::<u32>(memory, pointer, ccountry::ID)? != 0 {
            return Err("event feed reserved country ID differs");
        }
        ccountry::Country {
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
    // Country equality checks raw references first, then two nonzero links.
    Ok(a == b
        || (a != 0
            && b != 0
            && countries.resolve(a)?.canonical_slot == countries.resolve(b)?.canonical_slot))
}

fn diplomacy(
    memory: &impl Memory,
    base: usize,
    country: ccountry::Country,
) -> Result<usize, &'static str> {
    let diplomacy = pointer(memory, country.pointer, ccountry::DIPLOMACY_STATUS)?;
    verify_vtable(memory, base, diplomacy, cdiplomacy_status::VTABLE_RVA)?;
    if pointer(memory, diplomacy, cdiplomacy_status::COUNTRY)? != country.pointer {
        return Err("event feed diplomacy owner mismatch");
    }
    Ok(diplomacy)
}

fn status(
    memory: &impl Memory,
    base: usize,
    countries: &ccountry::Reader<'_, impl Memory>,
    owner: ccountry::Country,
    target: ccountry::Country,
    target_reference: u32,
) -> Result<usize, &'static str> {
    let diplomacy = diplomacy(memory, base, owner)?;
    let matrix = Array::read(memory, at(diplomacy, cdiplomacy_status::RELATIONS)?, 8)
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

/// Returns of the declaration, join and civil-war transfer calls into war creation.
const DECLARATION: Rva = Rva(0x00d9_8a1e);
const JOIN: Rva = Rva(0x00d9_ed0a);
const TRANSFER: Rva = Rva(0x0116_8728);
const WAR_FRAME_RETURNS: [Rva; 2] = [Rva(0x00d8_1bff), Rva(0x00d8_167d)];

/// Every frame and producer key the war-source classifier distinguishes resolves, or the first
/// withheld one's reason.
pub fn war_source_sites(base: usize) -> Result<(), &'static str> {
    engine_sites::require(base, &WAR_FRAME_RETURNS)?;
    engine_sites::require(base, &[DECLARATION, JOIN, TRANSFER])
}

fn war_source(memory: &impl Memory, base: usize, frame: WarFrame) -> Result<Source, &'static str> {
    // Include return-address space for every intervening call when validating
    // the three nested frame extents.
    if value::<u64>(memory, frame.rsp_before_call, 0x1d8)? != site(base, WAR_FRAME_RETURNS[0])? as u64
        || value::<u64>(memory, frame.rsp_before_call, 0x448)?
            != site(base, WAR_FRAME_RETURNS[1])? as u64
        || pointer(memory, frame.rsp_before_call, 0x438)? != frame.rcx
    {
        return Err("event feed war creator frame mismatch");
    }
    let source_pc = pointer(memory, frame.rsp_before_call, 0x4a8)?;
    // Declaration, join and transfer routes have independent caller identities.
    match engine_sites::which(base, source_pc, &[DECLARATION, JOIN, TRANSFER]) {
        Some(DECLARATION) => Ok(Source::Declaration),
        Some(JOIN) => Ok(Source::Join),
        Some(TRANSFER) => Ok(Source::CivilWarWarTransfer),
        _ => Err("event feed war producer is unclassified"),
    }
}

/// Copy before the second replacement. No pre-null requirement: native may
/// replace an old relation whose destructor reenters. The same candidate
/// already installed in the selected slot is an unproved-source refusal.
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
    verify_vtable(memory, base, owner_diplomacy, cdiplomacy_status::VTABLE_RVA)?;
    let owner = pointer(memory, owner_diplomacy, cdiplomacy_status::COUNTRY)?;
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

/// Only on a proven normal return, before resuming the original caller.
/// Replacement destroys the prior relation before storing the candidate.
/// Fresh reads retain full country IDs, raw candidate pair and that exact
/// candidate in both matrix slots. Never retry or substitute a relation.
/// Root-address equality cannot authenticate world lifetime.
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

/// Copy the annex operation before authored dispatch can mutate either role.
/// Zero country references are preserved through the reserved native object.
/// Escape handling and partial observations belong to the enclosing runtime owner.
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
