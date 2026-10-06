//! Callback-local demand for border-event targeted-decision descriptions.
//!
//! The 1413D69D0 marker resolves each border side's win/lose identifier and
//! describes immediate/single-option bodies only when option count is below
//! two. Branches and scope selections are conservatively unioned: recording a
//! matching decision for another country is cheap; executing effects is wrong.
//! Unsupported effect classes are not described by the viewer, so their
//! subtrees demand nothing. Native pointers never survive the capture callback.

use crate::{
    Memory, cevent, cgame_state, cland_border_war_combat as combat,
    cland_border_war_combatant as side, cland_combat, cland_combatant, cnaval_combat,
    cpdx_array::Array, creference_registry, event_namespace as namespace, msvc_string, read,
    read_slice, walk::Outcome,
};

/// CEvent/CEventOption RTTI; 14116A440/14116A610 constructors.
const EVENT_VTABLE: usize = 0x0298_2fb8;
const OPTION_VTABLE: usize = 0x0298_2f18;
/// 1413D69D0 packs type 0x32 with the resolved event identifier.
const EVENT_TYPE: u32 = 0x32;
/// 14116B330 selects the embedded option; 141527890 selects its CEffect.
const IMMEDIATE: usize = 0x118;
const OPTION_EFFECT: usize = 0x120;
/// CEffect RTTI and CIfEffect's attached else, read by 141381970.
const EFFECT_VTABLE: usize = 0x027b_e4c0;
const IF_VTABLE: usize = 0x027b_e590;
const ELSE: usize = 0xb0;
/// CRemoveTargetedDecisionEffect RTTI; 14038BF60 resolves its name at +58.
const REMOVE_VTABLE: usize = 0x0275_e4c0;
const DECISION_NAME: usize = 0x58;

/// The supported description classes' pinned RTTI vtables, all CEffect bases.
/// These are executable facts, not authored effect keys or a content census.
const DESCRIBED: &[usize] = &[
    EFFECT_VTABLE,
    IF_VTABLE,
    REMOVE_VTABLE,
    0x0275_1ff8, // CAddPoliticalPowerEffect: 140376050.
    0x0275_9db8, // CAddArmyExperienceEffect: 140370FF0.
    0x0275_8bf8, // CAddStabilityEffect: 140377DF0.
    0x0275_8dc8, // CAddWarSupportEffect: 1403797B0.
    0x0275_35f8, // CAddStateClaimEffect: 140378010.
    0x0275_31f8, // CTransferStateEffect: 140398680.
    0x0276_02d0, // CStartBorderWarEffect: 140395AE0.
    0x0275_8f98, // CDeclareWarEffect: 14037E9B0.
    0x0275_6b70, // CAnnexCountryEffect: 1403799D0.
    0x027b_2a08, // CAddToFactionEffect: 1404AC450.
    0x027b_b520, // NDoctrines::CAddMasteryEffect: 14051D460.
    0x0273_bc98, // CAddDoctrineCostReduction: 14031FCD0.
    0x0275_1c20, // CAddTimedIdeaEffect: 140378970.
    0x0275_6450, // CCountryEventEffect: 14037BDC0 (title only).
    0x0276_0fa8, // CDeleteTemplateAndItsUnits: 14037EC90.
    0x0275_f568, // CClearVariableEffect: 140176EE0.
    0x0275_61c8, // CSetCountryFlagEffect: 140176EE0.
    0x0275_6378, // CClearCountryFlagEffect: 140176EE0.
    0x0273_7a40, // CClearStateFlagEffect: 140176EE0.
    0x0275_a188, // CSaveEventTargetAsEffect: 14038D670.
    0x0299_f6f8, // CContextEffect: 141381200.
    0x0275_76e8, // CEveryStateEffect: 140312880.
    0x0299_fcb8, // CHiddenEffect: 1413818F0.
    0x0299_fd88, // CEffectTooltipEffect: 140532780.
    0x0299_ff28, // CCustomEffectTooltipEffect: 1413832B0.
];

/// Validated native bytes, usable only inside this quiescent callback.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Token {
    pub address: usize,
    pub len: usize,
}

/// A name copied once into the callback's caller-owned byte arena.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Name {
    pub start: usize,
    pub len: usize,
}

#[derive(Clone, Copy)]
pub struct Demand<'a> {
    pub names: &'a [Name],
    pub bytes: &'a [u8],
}

pub fn copy_names(
    memory: &impl Memory,
    tokens: &[Token],
    names: &mut [Name],
    bytes: &mut [u8],
) -> Result<usize, &'static str> {
    let required = tokens.iter().try_fold(0_usize, |total, token| {
        total
            .checked_add(token.len)
            .ok_or("border demand byte count overflow")
    })?;
    if tokens.len() > names.len() || required > bytes.len() {
        return Err("border demand name capacity");
    }
    let mut start = 0;
    for (&token, name) in tokens.iter().zip(names) {
        let end = start + token.len;
        if !read_slice(memory, token.address, &mut bytes[start..end]) {
            return Err("border demand token bytes unreadable");
        }
        *name = Name {
            start,
            len: token.len,
        };
        start = end;
    }
    Ok(required)
}

pub fn matches_name(name: Name, bytes: &[u8], wanted: &[u8]) -> Result<bool, &'static str> {
    let source = name
        .start
        .checked_add(name.len)
        .and_then(|end| bytes.get(name.start..end))
        .ok_or("border demand name span invalid")?;
    for (index, actual) in source.iter().copied().chain(std::iter::once(0)).enumerate() {
        let wanted = wanted.get(index).copied().unwrap_or(0);
        if !actual.is_ascii() || !wanted.is_ascii() {
            return Err("border demand name comparison locale unavailable");
        }
        if !actual.eq_ignore_ascii_case(&wanted) {
            return Ok(false);
        }
        if actual == 0 {
            return Ok(true);
        }
    }
    unreachable!("name has a terminating zero")
}

/// Reusable DFS storage; no allocation or arbitrary recursion limit.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Frame {
    effect: usize,
    children: Array,
    next: u32,
    attached_else: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Walk {
    pub outcome: Outcome,
    pub len: usize,
    pub required_rows: usize,
    /// Largest observed depth, or the next required depth on stack shortage.
    /// A shortage withholds the demand census; grow off-thread and try next hour.
    pub required_stack: usize,
    pub required_events: usize,
}

impl Default for Walk {
    fn default() -> Self {
        Self {
            outcome: Outcome::Rows { len: 0 },
            len: 0,
            required_rows: 0,
            required_stack: 0,
            required_events: 0,
        }
    }
}

impl Walk {
    pub fn complete(self, output: &[Token], stack: &[Frame]) -> bool {
        matches!(self.outcome, Outcome::Rows { .. })
            && self.required_rows <= output.len()
            && self.required_stack <= stack.len()
    }
}

/// Demand tokens only from active border combats; ordinary combat participants
/// are never read. A native read fault is an unavailable census, never no demand.
pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut [Token],
    stack: &mut [Frame],
    events: &mut [u32],
) -> Walk {
    let mut reader = Reader {
        memory,
        base,
        output,
        stack,
        events,
        events_len: 0,
        result: Walk::default(),
    };
    let acquired = acquire(&mut reader, game_state);
    reader.result.outcome = match acquired {
        Err(reason) => Outcome::Unavailable { reason },
        Ok(()) if reader.result.required_stack > reader.stack.len() => Outcome::Unavailable {
            reason: "border demand stack capacity",
        },
        Ok(()) if reader.result.required_rows > reader.output.len() => Outcome::Truncated {
            len: reader.result.len,
            unread_rows: (reader.result.required_rows - reader.result.len)
                .try_into()
                .unwrap_or(u32::MAX),
        },
        Ok(()) => Outcome::Rows {
            len: reader.result.len,
        },
    };
    reader.result
}

fn acquire(reader: &mut Reader<'_, impl Memory>, game_state: usize) -> Result<(), &'static str> {
    let memory = reader.memory;
    let combats = reader.array(at(game_state, cgame_state::COMBATS + 8)?)?;
    for ordinal in 0..combats.count {
        let pointer = reader.pointer(combats, ordinal)?;
        match reader.vtable(pointer)? {
            cland_combat::VTABLE_RVA | cnaval_combat::VTABLE_RVA => continue,
            combat::VTABLE_RVA => {}
            // This census needs only border-war classes. Other combat classes
            // cannot supply a border side's win/lose events.
            _ => continue,
        }
        let identity = field::<[u8; 16]>(memory, pointer, cland_combat::BODY)?;
        if word(&identity[..8]) != at(reader.base, combat::BODY_VTABLE_RVA)? as u64
            || word(&identity[8..]) == 0
        {
            return Err("border demand combat identity invalid");
        }
        let sides = field::<[u8; 16]>(memory, pointer, cland_combat::ATTACKER)?;
        if sides[..8] == sides[8..] {
            return Err("border demand combat sides coincide");
        }
        for (index, bytes) in sides.chunks_exact(8).enumerate() {
            let combatant =
                usize::try_from(word(bytes)).map_err(|_| "border demand pointer overflow")?;
            if reader.vtable(combatant)? != side::VTABLE_RVA
                || field::<u64>(memory, combatant, cland_combatant::COMBAT)? != pointer as u64
                || field::<u8>(memory, combatant, cland_combatant::IS_ATTACKER)?
                    != (1 - index) as u8
            {
                return Err("border demand side identity invalid");
            }
            for offset in [side::WIN_EVENT, side::LOSE_EVENT] {
                let name = token(memory, at(combatant, offset)?)?;
                if name.len != 0 {
                    reader.event(name)?;
                }
            }
        }
    }
    Ok(())
}

/// Native _stricmp name comparison (14253D9F8), through the first NUL.
/// A non-ASCII comparison needs the recording client's unrecorded locale.
pub fn matches(memory: &impl Memory, token: Token, bytes: &[u8]) -> Result<bool, &'static str> {
    let mut chunk = [0; 128];
    let mut first = 0;
    loop {
        let remaining = token
            .len
            .checked_sub(first)
            .and_then(|len| len.checked_add(1))
            .ok_or("border demand name extent overflow")?;
        let count = remaining.min(chunk.len());
        if !read_slice(memory, at(token.address, first)?, &mut chunk[..count]) {
            return Err("border demand token bytes unreadable");
        }
        for (index, &actual) in chunk[..count].iter().enumerate() {
            let wanted = bytes.get(first + index).copied().unwrap_or(0);
            if !actual.is_ascii() || !wanted.is_ascii() {
                return Err("border demand name comparison locale unavailable");
            }
            if !actual.eq_ignore_ascii_case(&wanted) {
                return Ok(false);
            }
            if actual == 0 {
                return Ok(true);
            }
        }
        first = first
            .checked_add(count)
            .ok_or("border demand name index overflow")?;
    }
}

struct Reader<'a, M> {
    memory: &'a M,
    base: usize,
    output: &'a mut [Token],
    stack: &'a mut [Frame],
    events: &'a mut [u32],
    events_len: usize,
    result: Walk,
}

impl<M: Memory> Reader<'_, M> {
    fn array(&self, address: usize) -> Result<Array, &'static str> {
        Array::read(self.memory, address, 8).ok_or("border demand array unreadable or invalid")
    }

    fn pointer(&self, array: Array, ordinal: u32) -> Result<usize, &'static str> {
        let address = array
            .element(ordinal, 8)
            .ok_or("border demand array index invalid")?;
        let pointer =
            read::<u64>(self.memory, address).ok_or("border demand pointer unreadable")?;
        usize::try_from(pointer)
            .ok()
            .filter(|p| *p != 0)
            .ok_or("border demand pointer invalid")
    }

    fn vtable(&self, pointer: usize) -> Result<usize, &'static str> {
        let vtable = read::<u64>(self.memory, pointer).ok_or("border demand vtable unreadable")?;
        usize::try_from(vtable)
            .ok()
            .and_then(|p| p.checked_sub(self.base))
            .ok_or("border demand vtable invalid")
    }

    fn event(&mut self, name: Token) -> Result<(), &'static str> {
        let Some(id) = event_id(self.memory, self.base, name)? else {
            return Ok(());
        };
        if self.events[..self.events_len].contains(&id) {
            return Ok(());
        }
        self.result.required_events = self
            .events_len
            .checked_add(1)
            .ok_or("border demand event count overflow")?;
        let destination = self
            .events
            .get_mut(self.events_len)
            .ok_or("border demand event capacity")?;
        *destination = id;
        self.events_len += 1;
        let handle = (u64::from(id) << 32) | u64::from(EVENT_TYPE);
        let Some(event) = creference_registry::resolve(self.memory, self.base, handle)? else {
            return Ok(());
        };
        if self.vtable(event)? != EVENT_VTABLE {
            return Err("border demand event class invalid");
        }
        if field::<u64>(self.memory, event, 8)? != handle {
            return Err("border demand event identity invalid");
        }
        // Operative events retain their named viewer refusal.
        if field::<u8>(self.memory, event, cevent::OPERATIVE_CHROME)? != 0 {
            return Ok(());
        }
        let options = self.array(at(event, cevent::OPTIONS)?)?;
        if options.count >= 2 {
            return Ok(());
        }
        let immediate = at(event, IMMEDIATE)?;
        if self.vtable(immediate)? != OPTION_VTABLE {
            return Err("border demand immediate class invalid");
        }
        self.effects(at(immediate, OPTION_EFFECT)?)?;
        if options.count == 1 {
            let option = self.pointer(options, 0)?;
            if self.vtable(option)? != OPTION_VTABLE {
                return Err("border demand option class invalid");
            }
            self.effects(at(option, OPTION_EFFECT)?)?;
        }
        Ok(())
    }

    fn push(&mut self, effect: usize, depth: usize) -> Result<bool, &'static str> {
        if self.stack[..depth]
            .iter()
            .any(|frame| frame.effect == effect)
        {
            return Err("border demand effect cycle");
        }
        let vtable = self.vtable(effect)?;
        if !DESCRIBED.contains(&vtable) {
            return Ok(false);
        }
        let required = depth.checked_add(1).ok_or("border demand depth overflow")?;
        self.result.required_stack = self.result.required_stack.max(required);
        if depth == self.stack.len() {
            return Ok(false);
        }
        if vtable == REMOVE_VTABLE {
            let name = token(self.memory, at(effect, DECISION_NAME)?)?;
            self.result.required_rows = self
                .result
                .required_rows
                .checked_add(1)
                .ok_or("border demand row count overflow")?;
            if let Some(slot) = self.output.get_mut(self.result.len) {
                *slot = name;
                self.result.len += 1;
            }
        }
        let children = self.array(at(effect, 8)?)?;
        let attached_else = if vtable == IF_VTABLE {
            field::<u64>(self.memory, effect, ELSE)? as usize
        } else {
            0
        };
        self.stack[depth] = Frame {
            effect,
            children,
            next: 0,
            attached_else,
        };
        Ok(true)
    }

    fn effects(&mut self, root: usize) -> Result<(), &'static str> {
        if !self.push(root, 0)? {
            return Ok(());
        }
        let mut depth = 1;
        while depth != 0 {
            let frame = self.stack[depth - 1];
            let next = if frame.next < frame.children.count {
                self.stack[depth - 1].next += 1;
                self.pointer(frame.children, frame.next)?
            } else if frame.attached_else != 0 {
                self.stack[depth - 1].attached_else = 0;
                frame.attached_else
            } else {
                depth -= 1;
                continue;
            };
            if self.push(next, depth)? {
                depth += 1;
            }
        }
        Ok(())
    }
}

fn at(address: usize, offset: usize) -> Result<usize, &'static str> {
    address
        .checked_add(offset)
        .ok_or("border demand address overflow")
}

fn field<T: crate::memory::Raw>(
    memory: &impl Memory,
    object: usize,
    offset: usize,
) -> Result<T, &'static str> {
    read(memory, at(object, offset)?).ok_or("border demand field unreadable")
}

fn word(bytes: &[u8]) -> u64 {
    u64::from_le_bytes(bytes.try_into().unwrap())
}

fn token(memory: &impl Memory, address: usize) -> Result<Token, &'static str> {
    let header =
        read::<[u8; 32]>(memory, address).ok_or("border demand string header unreadable")?;
    let len = word(&header[16..24]);
    let capacity = word(&header[24..]);
    if len > capacity || capacity < msvc_string::INLINE_CAPACITY {
        return Err("border demand string header invalid");
    }
    let len = usize::try_from(len).map_err(|_| "border demand string length overflow")?;
    let address = if capacity == msvc_string::INLINE_CAPACITY {
        address
    } else {
        usize::try_from(word(&header[..8])).map_err(|_| "border demand string pointer overflow")?
    };
    if address == 0 || read::<u8>(memory, at(address, len)?) != Some(0) {
        return Err("border demand string terminator invalid");
    }
    Ok(Token { address, len })
}

/// Passive 1401F03A0 resolution, preserving suffix wrapping at 1424B8700.
fn event_id(memory: &impl Memory, base: usize, name: Token) -> Result<Option<u32>, &'static str> {
    let mut identifier = [0; 128];
    let buffered = name.len <= identifier.len();
    let mut dot = None;
    let mut hash = 0x811c_9dc5_u32;
    let mut prefix_hash = hash;
    let mut ascii = true;
    let mut prefix_ascii = true;
    let mut suffix = 0_i32;
    for first in (0..name.len).step_by(identifier.len()) {
        let count = (name.len - first).min(identifier.len());
        if !read_slice(memory, at(name.address, first)?, &mut identifier[..count]) {
            return Err("border demand event identifier unreadable");
        }
        for (index, &byte) in identifier[..count].iter().enumerate() {
            if byte == b'.' {
                dot = Some(first + index);
                prefix_hash = hash;
                prefix_ascii = ascii;
                suffix = 0;
            } else if dot.is_some() {
                suffix = suffix
                    .wrapping_mul(10)
                    .wrapping_add(i32::from(byte as i8))
                    .wrapping_sub(0x30);
            }
            ascii &= byte.is_ascii();
            hash = (hash ^ u32::from(byte.to_ascii_lowercase())).wrapping_mul(0x0100_0193);
        }
    }
    let Some(dot) = dot.filter(|index| *index != 0) else {
        return Ok(None);
    };
    if name.len - dot - 1 > i32::MAX as usize {
        return Err("border demand suffix length invalid");
    }
    if !prefix_ascii {
        return Err("border demand namespace comparison locale unavailable");
    }
    let namespaces = Array::read(memory, at(base, namespace::TABLE_RVA)?, namespace::STRIDE)
        .ok_or("border demand namespace array unreadable or invalid")?;
    for ordinal in 0..namespaces.count {
        let row = namespaces
            .element(ordinal, namespace::STRIDE)
            .ok_or("border demand namespace index invalid")?;
        if field::<u32>(memory, row, namespace::NAME_HASH)? != prefix_hash {
            continue;
        }
        let registered = token(memory, row)?;
        if registered.len != dot {
            continue;
        }
        let mut equal = true;
        let mut registered_bytes = [0; 128];
        for first in (0..dot).step_by(registered_bytes.len()) {
            let count = (dot - first).min(registered_bytes.len());
            if !read_slice(
                memory,
                at(registered.address, first)?,
                &mut registered_bytes[..count],
            ) {
                return Err("border demand namespace bytes unreadable");
            }
            if !buffered && !read_slice(memory, at(name.address, first)?, &mut identifier[..count])
            {
                return Err("border demand namespace bytes unreadable");
            }
            let wanted = if buffered {
                &identifier[first..first + count]
            } else {
                &identifier[..count]
            };
            if !registered_bytes[..count].eq_ignore_ascii_case(wanted) {
                equal = false;
                break;
            }
        }
        if !equal {
            continue;
        }
        let namespace = ordinal
            .wrapping_add(namespace::ID_FIRST_NAMESPACE_SLOT)
            .wrapping_mul(namespace::ID_NAMESPACE_SPAN) as i32;
        return Ok((namespace >= 0).then_some(namespace.wrapping_add(suffix) as u32));
    }
    Ok(None)
}

#[cfg(test)]
mod tests;
