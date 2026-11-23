
use engine_sites::Rva;
use engine_native_memory::{
    Memory,
    cpdx_array::Array,
    msvc_string,
    read,
    read_slice,
    walk::Outcome,
};
use crate::{cevent, event_namespace as namespace};
use engine_native::{cgame_state, cland_combat, cland_combatant, cnaval_combat};
use engine_native_parts::{
    cland_border_war_combat as combat,
    cland_border_war_combatant as side,
    creference_registry,
};

const EVENT_VTABLE: Rva = Rva(0x02a1_6008);
const OPTION_VTABLE: Rva = Rva(0x02a1_5f68);
const EVENT_TYPE: u32 = 0x32;
const IMMEDIATE: usize = 0x118;
const OPTION_EFFECT: usize = 0x120;
const EFFECT_VTABLE: Rva = Rva(0x0285_26e0);
const IF_VTABLE: Rva = Rva(0x0285_27b0);
const ELSE: usize = 0xb0;
const REMOVE_VTABLE: Rva = Rva(0x027f_1ff0);
const DECISION_NAME: usize = 0x58;

const DESCRIBED: &[Rva] = &[
    EFFECT_VTABLE,
    IF_VTABLE,
    REMOVE_VTABLE,
    Rva(0x027e_5b28),
    Rva(0x027e_d8e8),
    Rva(0x027e_c728),
    Rva(0x027e_c8f8),
    Rva(0x027e_7128),
    Rva(0x027e_6d28),
    Rva(0x027f_3e00),
    Rva(0x027e_cac8),
    Rva(0x027e_a6a0),
    Rva(0x0284_6368),
    Rva(0x0284_faa0),
    Rva(0x027c_f708),
    Rva(0x027e_5750),
    Rva(0x027e_9f80),
    Rva(0x027f_4b88),
    Rva(0x027f_3098),
    Rva(0x027e_9cf8),
    Rva(0x027e_9ea8),
    Rva(0x027c_b558),
    Rva(0x027e_dcb8),
    Rva(0x02a3_1830),
    Rva(0x027e_b218),
    Rva(0x02a3_1d78),
    Rva(0x02a3_1e48),
    Rva(0x02a3_1fe8),
];

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Token {
    pub address: usize,
    pub len: usize,
}

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

pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<Token>,
    stack: &mut Vec<Frame>,
    events: &mut Vec<u32>,
) -> Walk {
    walk_buffer(
        memory,
        base,
        game_state,
        engine_native_memory::walk::Buffer::Growing(output),
        engine_native_memory::walk::Buffer::Growing(stack),
        engine_native_memory::walk::Buffer::Growing(events),
    )
}

pub fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: engine_native_memory::walk::Buffer<'_, Token>,
    stack: engine_native_memory::walk::Buffer<'_, Frame>,
    events: engine_native_memory::walk::Buffer<'_, u32>,
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
        let vtable = reader.vtable(pointer)?;
        match reader.class(
            vtable,
            &[cland_combat::VTABLE_RVA, cnaval_combat::VTABLE_RVA, combat::VTABLE_RVA],
        )? {
            Some(cland_combat::VTABLE_RVA | cnaval_combat::VTABLE_RVA) => continue,
            Some(combat::VTABLE_RVA) => {}
            _ => continue,
        }
        let identity = field::<[u8; 16]>(memory, pointer, cland_combat::BODY)?;
        if word(&identity[..8]) != reader.key(combat::BODY_VTABLE_RVA)? as u64
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
            if reader.vtable(combatant)? != reader.key(side::VTABLE_RVA)?
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
    output: engine_native_memory::walk::Buffer<'a, Token>,
    stack: engine_native_memory::walk::Buffer<'a, Frame>,
    events: engine_native_memory::walk::Buffer<'a, u32>,
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
            .filter(|&p| p >= self.base)
            .ok_or("border demand vtable invalid")
    }

    fn key(&self, key: Rva) -> Result<usize, &'static str> {
        engine_sites::at(self.base, key).ok_or("border demand vtable withheld")
    }

    fn class(&self, vtable: usize, candidates: &[Rva]) -> Result<Option<Rva>, &'static str> {
        engine_sites::classify(self.base, vtable, candidates)
            .map_err(|_| "border demand vtable withheld")
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
        self.events.ensure(self.result.required_events)?;
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
        if self.vtable(event)? != self.key(EVENT_VTABLE)? {
            return Err("border demand event class invalid");
        }
        if field::<u64>(self.memory, event, 8)? != handle {
            return Err("border demand event identity invalid");
        }
        if field::<u8>(self.memory, event, cevent::OPERATIVE_CHROME)? != 0 {
            return Ok(());
        }
        let options = self.array(at(event, cevent::OPTIONS)?)?;
        if options.count >= 2 {
            return Ok(());
        }
        let immediate = at(event, IMMEDIATE)?;
        if self.vtable(immediate)? != self.key(OPTION_VTABLE)? {
            return Err("border demand immediate class invalid");
        }
        self.effects(at(immediate, OPTION_EFFECT)?)?;
        if options.count == 1 {
            let option = self.pointer(options, 0)?;
            if self.vtable(option)? != self.key(OPTION_VTABLE)? {
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
        let Some(vtable) = self.class(self.vtable(effect)?, DESCRIBED)? else {
            return Ok(false);
        };
        let required = depth.checked_add(1).ok_or("border demand depth overflow")?;
        self.result.required_stack = self.result.required_stack.max(required);
        self.stack.ensure(required)?;
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
            self.output.ensure(self.result.required_rows)?;
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

fn field<T: engine_native_memory::memory::Raw>(
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
    let table = engine_sites::at(base, namespace::TABLE_RVA).ok_or("border demand address overflow")?;
    let namespaces = Array::read(memory, table, namespace::STRIDE)
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
