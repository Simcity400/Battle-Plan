use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write as _,
    io,
};

use iced_x86::{Decoder, DecoderOptions, FlowControl, Instruction, Mnemonic, OpKind, Register};

use super::{keywords::KeywordTables, leak_string, pe::PeImage, verify_digest};

const TRIGGER_REGISTRATION: u32 = 0x0059_7b90;
const EFFECT_REGISTRATION: u32 = 0x0058_88e0;
const OBJECT_ALLOCATION: u32 = 0x025a_009c;
const ENTRY_FACTORY: u32 = 8;
const TRIGGER_VALUE_READER: u32 = 40;
const TRIGGER_KEY_READER: u32 = 48;
const TRIGGER_SCOPE_MASK: u32 = 112;
const EFFECT_VALUE_READER: u32 = 24;
const EFFECT_KEY_READER: u32 = 32;
const ASSIGN_READER: u32 = 0x0144_6ac0;
const COMPARISON_READER: u32 = 0x0144_5e70;
const BODY_READER: u32 = 0x0059_6f30;
const CONTEXT_EFFECT_VTABLE: u32 = 0x02a3_1110;
const SCRIPTED_EFFECT_VTABLE: u32 = 0x02a3_11e0;

const TRIGGERS_DIGEST: &str = "03c08b0bd5ce9ccba4b299391d917c1a985fb197984924900dadd81e6a68ed28";
const EFFECTS_DIGEST: &str = "5bc2422fcf914a6cfdf3dd9a3edefdcb74d30d6d20d584f7c89bf249b5b21473";
const TRIGGER_CLASSES_DIGEST: &str =
    "8593a4decfe245219b40614a3ff4c3d9b2c937023d5d8192abdf18208efa6084";
const EFFECT_READERS_DIGEST: &str =
    "6153bad5a9a2490702382501adcc3d5436d46870d4e8f8e874debd8b655baa08";
const EFFECT_IDENTITIES_DIGEST: &str =
    "763b7d3b25b3bc6da1b49bbda3caecac91ab4f2078164fd79d7f90dd1964d2f0";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TriggerReader {
    Assign,
    Comparison,
    Body,
    Own(u64),
}

#[derive(Debug, Default)]
pub struct RegistryTables {
    pub triggers: Vec<(u32, &'static str)>,
    pub trigger_classes: Vec<(u32, u32, TriggerReader)>,
    pub trigger_key_readers: Vec<(u32, u64)>,
    pub effects: Vec<(u32, &'static str)>,
    pub effect_classes: Vec<(u32, u64, u64)>,
    pub effect_identities: Vec<(u32, &'static str, u64)>,
    context_effect_names: BTreeMap<u64, &'static str>,
}

impl RegistryTables {
    pub fn effect_class_name(&self, vtable: u64) -> Option<&'static str> {
        self.effect_identities
            .iter()
            .find_map(|&(_, name, identity)| (identity == vtable).then_some(name))
            .or_else(|| self.context_effect_names.get(&vtable).copied())
    }

    pub(super) fn decode(image: &PeImage, keywords: &KeywordTables) -> io::Result<Self> {
        let [trigger_sites, effect_sites] =
            call_sites(image, [TRIGGER_REGISTRATION, EFFECT_REGISTRATION])?;
        let triggers = registrations(image, TRIGGER_REGISTRATION, trigger_sites, "triggers")?;
        let effects = registrations(image, EFFECT_REGISTRATION, effect_sites, "effects")?;
        let mut result = Self::default();
        let mut factories = Factories::new(image);
        for (token, entry) in triggers {
            let name = keyword(keywords, token, "triggers")?;
            let object = factories.object(entry).map_err(|error| {
                invalid(format!(
                    "triggers: keyword {token}: {error} (1 unsupported occurrence)"
                ))
            })?;
            let reader = code_pointer(image, checked_add(object, TRIGGER_VALUE_READER)?)?;
            let reader_rva = image.va_to_rva(reader)?;
            let reader = match reader_rva {
                ASSIGN_READER => TriggerReader::Assign,
                COMPARISON_READER => TriggerReader::Comparison,
                BODY_READER => TriggerReader::Body,
                _ => TriggerReader::Own(reader),
            };
            let mask_reader = code_pointer(image, checked_add(object, TRIGGER_SCOPE_MASK)?)?;
            let mask = constant_return(image, image.va_to_rva(mask_reader)?)?;
            let key = code_pointer(image, checked_add(object, TRIGGER_KEY_READER)?)?;
            result.triggers.push((token, name));
            result.trigger_classes.push((token, mask, reader));
            result.trigger_key_readers.push((token, key));
        }
        for (token, entry) in effects {
            let name = keyword(keywords, token, "effects")?;
            let object = factories.object(entry).map_err(|error| {
                invalid(format!(
                    "effects: keyword {token}: {error} (1 unsupported occurrence)"
                ))
            })?;
            let value = code_pointer(image, checked_add(object, EFFECT_VALUE_READER)?)?;
            let key = code_pointer(image, checked_add(object, EFFECT_KEY_READER)?)?;
            let class_name = leak_string(class_name(image, object)?);
            result.effects.push((token, name));
            result.effect_classes.push((token, value, key));
            result
                .effect_identities
                .push((token, class_name, image.base + u64::from(object)));
        }
        for vtable in [CONTEXT_EFFECT_VTABLE, SCRIPTED_EFFECT_VTABLE] {
            let name = leak_string(class_name(image, vtable)?);
            result
                .context_effect_names
                .insert(image.base + u64::from(vtable), name);
        }
        result.verify()?;
        Ok(result)
    }

    fn verify(&self) -> io::Result<()> {
        verify_names("triggers", &self.triggers, 596, TRIGGERS_DIGEST)?;
        verify_names("effects", &self.effects, 553, EFFECTS_DIGEST)?;
        let mut text = String::new();
        for &(token, mask, reader) in &self.trigger_classes {
            let reader = match reader {
                TriggerReader::Assign => "Assign".to_owned(),
                TriggerReader::Comparison => "Comparison".to_owned(),
                TriggerReader::Body => "Body".to_owned(),
                TriggerReader::Own(address) => format!("Own({address})"),
            };
            writeln!(text, "{token}\t{mask}\t{reader}").unwrap();
        }
        verify_count("trigger classes", self.trigger_classes.len(), 596)?;
        verify_digest("trigger classes", text.as_bytes(), TRIGGER_CLASSES_DIGEST)?;
        text.clear();
        for &(token, value, key) in &self.effect_classes {
            writeln!(text, "{token}\t{value}\t{key}").unwrap();
        }
        verify_count("effect readers", self.effect_classes.len(), 553)?;
        verify_digest("effect readers", text.as_bytes(), EFFECT_READERS_DIGEST)?;
        text.clear();
        for &(token, name, vtable) in &self.effect_identities {
            writeln!(text, "{token}\t{name}\t{vtable}").unwrap();
        }
        verify_count("effect identities", self.effect_identities.len(), 553)?;
        verify_digest(
            "effect identities",
            text.as_bytes(),
            EFFECT_IDENTITIES_DIGEST,
        )
    }
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn checked_add(rva: u32, offset: u32) -> io::Result<u32> {
    rva.checked_add(offset)
        .ok_or_else(|| invalid("registry address overflow"))
}

fn keyword(keywords: &KeywordTables, token: u32, table: &str) -> io::Result<&'static str> {
    keywords
        .by_id(token)
        .ok_or_else(|| invalid(format!("{table}: unknown keyword {token}")))
}

fn verify_count(table: &str, actual: usize, expected: usize) -> io::Result<()> {
    if actual == expected {
        Ok(())
    } else {
        Err(invalid(format!(
            "{table}: expected {expected} rows, found {actual}"
        )))
    }
}

fn verify_names(table: &str, rows: &[(u32, &str)], count: usize, digest: &str) -> io::Result<()> {
    verify_count(table, rows.len(), count)?;
    let mut text = String::new();
    for &(token, name) in rows {
        writeln!(text, "{token}\t{name}").unwrap();
    }
    verify_digest(table, text.as_bytes(), digest)
}

fn is_code(image: &PeImage, rva: u32) -> bool {
    image
        .executable_ranges()
        .iter()
        .any(|&(start, end)| start <= rva && rva < end)
}

fn code_pointer(image: &PeImage, rva: u32) -> io::Result<u64> {
    let address = image.u64(rva)?;
    if !is_code(image, image.va_to_rva(address)?) {
        return Err(invalid("registry reader is outside executable sections"));
    }
    Ok(address)
}

fn is_vtable(image: &PeImage, address: u64) -> bool {
    image
        .va_to_rva(address)
        .and_then(|rva| code_pointer(image, rva))
        .is_ok()
}

fn direct_target(image: &PeImage, instruction: &Instruction) -> io::Result<Option<u32>> {
    if matches!(instruction.mnemonic(), Mnemonic::Call | Mnemonic::Jmp)
        && matches!(
            instruction.op0_kind(),
            OpKind::NearBranch64 | OpKind::NearBranch32
        )
    {
        return image.va_to_rva(instruction.near_branch_target()).map(Some);
    }
    Ok(None)
}

/// The sites of every five-byte relative call or jump whose destination is one
/// of `targets`, by target, from one pass over the executable sections.
fn call_sites<const N: usize>(
    image: &PeImage,
    targets: [u32; N],
) -> io::Result<[BTreeSet<u32>; N]> {
    let mut sites = std::array::from_fn(|_| BTreeSet::new());
    for &(start, end) in image.executable_ranges() {
        let bytes = image.bytes(start, (end - start) as usize)?;
        // A site is the opcode byte of a whole five-byte instruction in the section.
        let Some(last) = bytes.len().checked_sub(5) else {
            continue;
        };
        for offset in memchr::memchr2_iter(0xE8, 0xE9, &bytes[..=last]) {
            let site = checked_add(
                start,
                u32::try_from(offset).map_err(|_| invalid("registry span overflow"))?,
            )?;
            let displacement = i32::from_le_bytes(bytes[offset + 1..offset + 5].try_into().unwrap());
            let destination = i64::from(site) + 5 + i64::from(displacement);
            for (&target, found) in targets.iter().zip(&mut sites) {
                if destination == i64::from(target) {
                    found.insert(site);
                }
            }
        }
    }
    Ok(sites)
}

fn registrations(
    image: &PeImage,
    target: u32,
    sites: BTreeSet<u32>,
    table: &str,
) -> io::Result<BTreeMap<u32, u32>> {
    let mut result = BTreeMap::new();
    let mut refused = 0;
    for site in sites {
        let row = (|| {
            let (start, end) = image.function(site)?;
            let instructions = image.instructions(start, end)?;
            let mut state = State::new();
            let mut boundary = false;
            for instruction in &instructions {
                let rva = image.va_to_rva(instruction.ip())?;
                if rva == site {
                    if direct_target(image, instruction)? != Some(target) {
                        return Err(invalid("registration is not an instruction boundary"));
                    }
                    boundary = true;
                    break;
                }
                state.simple(image, instruction)?;
                if let Some(call) = direct_target(image, instruction)?
                    && instruction.mnemonic() == Mnemonic::Call
                {
                    state.clobber();
                    if call == OBJECT_ALLOCATION {
                        state.allocate(rva);
                    }
                }
            }
            if !boundary {
                return Err(invalid("registration is not an instruction boundary"));
            }
            let Some(Value::Number(token)) = state.registers.get(&Register::RDX).copied() else {
                return Err(invalid("registration has no keyword argument"));
            };
            let Some(Value::Object(root, offset)) = state.registers.get(&Register::R8).copied()
            else {
                return Err(invalid("registration has no entry object"));
            };
            let entry = state
                .tables
                .get(&(root, offset))
                .copied()
                .ok_or_else(|| invalid("registration has no entry identity"))?;
            let token =
                u32::try_from(token).map_err(|_| invalid("registration keyword overflow"))?;
            Ok((token, entry))
        })();
        match row {
            Ok((token, entry)) => {
                if result
                    .insert(token, entry)
                    .is_some_and(|previous| previous != entry)
                {
                    return Err(invalid(format!(
                        "{table}: conflicting registration for keyword {token}"
                    )));
                }
            }
            Err(_) => refused += 1,
        }
    }
    if refused != 0 {
        return Err(invalid(format!(
            "{table}: {refused} unsupported registration occurrences"
        )));
    }
    Ok(result)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Value {
    Number(u64),
    Address(u64),
    Object(u32, i64),
}

#[derive(Clone, Default, PartialEq, Eq, PartialOrd, Ord)]
struct State {
    registers: BTreeMap<Register, Value>,
    tables: BTreeMap<(u32, i64), u32>,
    stack: BTreeMap<i64, Value>,
    stack_offset: i64,
}

impl State {
    fn new() -> Self {
        let mut result = Self::default();
        result.registers.insert(Register::RCX, Value::Object(0, 0));
        result
    }

    fn allocate(&mut self, root: u32) {
        self.registers.insert(Register::RAX, Value::Object(root, 0));
    }

    fn clobber(&mut self) {
        for register in [
            Register::RAX,
            Register::RCX,
            Register::RDX,
            Register::R8,
            Register::R9,
            Register::R10,
            Register::R11,
        ] {
            self.registers.remove(&register);
        }
    }

    fn operand(&self, instruction: &Instruction, operand: u32) -> Option<Value> {
        match instruction.op_kind(operand) {
            OpKind::Register => {
                let source = instruction.op_register(operand);
                match self.registers.get(&register(source)).copied()? {
                    Value::Number(number) => {
                        let bits = source.size() * 8;
                        Some(Value::Number(if bits < 64 {
                            number & ((1u64 << bits) - 1)
                        } else {
                            number
                        }))
                    }
                    value if is_pointer_register(source) => Some(value),
                    _ => None,
                }
            }
            OpKind::Immediate8 => Some(Value::Number(u64::from(instruction.immediate8()))),
            OpKind::Immediate16 => Some(Value::Number(u64::from(instruction.immediate16()))),
            OpKind::Immediate32 => Some(Value::Number(u64::from(instruction.immediate32()))),
            OpKind::Immediate64 => Some(Value::Number(instruction.immediate64())),
            OpKind::Immediate8to32 => {
                Some(Value::Number(instruction.immediate8to32() as u32 as u64))
            }
            OpKind::Immediate8to64 => Some(Value::Number(instruction.immediate8to64() as u64)),
            OpKind::Immediate32to64 => Some(Value::Number(instruction.immediate32to64() as u64)),
            OpKind::Memory
                if instruction.memory_base() == Register::RSP
                    && instruction.memory_index() == Register::None =>
            {
                self.stack
                    .get(&(self.stack_offset + instruction.memory_displacement64() as i64))
                    .copied()
            }
            _ => None,
        }
    }

    fn simple(&mut self, image: &PeImage, instruction: &Instruction) -> io::Result<()> {
        let mnemonic = instruction.mnemonic();
        if matches!(mnemonic, Mnemonic::Push | Mnemonic::Pop) {
            self.stack_offset += if mnemonic == Mnemonic::Push { -8 } else { 8 };
            if mnemonic == Mnemonic::Pop && instruction.op0_kind() == OpKind::Register {
                self.registers.remove(&register(instruction.op0_register()));
            }
            return Ok(());
        }
        if matches!(mnemonic, Mnemonic::Add | Mnemonic::Sub)
            && instruction.op0_kind() == OpKind::Register
            && instruction.op0_register() == Register::RSP
        {
            if let Some(Value::Number(amount)) = self.operand(instruction, 1) {
                let amount =
                    i64::try_from(amount).map_err(|_| invalid("constructor stack overflow"))?;
                self.stack_offset = self
                    .stack_offset
                    .checked_add(if mnemonic == Mnemonic::Add {
                        amount
                    } else {
                        -amount
                    })
                    .ok_or_else(|| invalid("constructor stack overflow"))?;
            }
            return Ok(());
        }
        if mnemonic == Mnemonic::Mov && instruction.op0_kind() == OpKind::Memory {
            let value = self.operand(instruction, 1);
            if instruction.memory_base() == Register::RSP
                && instruction.memory_index() == Register::None
            {
                let offset = self.stack_offset + instruction.memory_displacement64() as i64;
                if let Some(value) = value {
                    self.stack.insert(offset, value);
                } else {
                    self.stack.remove(&offset);
                }
            } else if instruction.memory_index() == Register::None
                && let Some(Value::Address(address)) = value
                && instruction.op1_kind() == OpKind::Register
                && is_pointer_register(instruction.op1_register())
                && is_vtable(image, address)
            {
                let Some(Value::Object(root, offset)) = self
                    .registers
                    .get(&register(instruction.memory_base()))
                    .copied()
                else {
                    return Err(invalid("constructor identity has an unknown destination"));
                };
                let offset = offset
                    .checked_add(instruction.memory_displacement64() as i64)
                    .ok_or_else(|| invalid("constructor object offset overflow"))?;
                self.tables
                    .insert((root, offset), image.va_to_rva(address)?);
            }
            return Ok(());
        }
        if instruction.op_count() == 0 || instruction.op0_kind() != OpKind::Register {
            return Ok(());
        }
        let destination = register(instruction.op0_register());
        let value = match mnemonic {
            Mnemonic::Lea if instruction.op1_kind() == OpKind::Memory => {
                if instruction.is_ip_rel_memory_operand() {
                    Some(Value::Address(instruction.ip_rel_memory_address()))
                } else if instruction.memory_index() == Register::None {
                    match self
                        .registers
                        .get(&register(instruction.memory_base()))
                        .copied()
                    {
                        Some(Value::Object(root, offset)) => offset
                            .checked_add(instruction.memory_displacement64() as i64)
                            .map(|offset| Value::Object(root, offset)),
                        _ => None,
                    }
                } else {
                    None
                }
            }
            Mnemonic::Mov => self.operand(instruction, 1),
            Mnemonic::Xor
                if instruction.op1_kind() == OpKind::Register
                    && instruction.op0_register() == instruction.op1_register() =>
            {
                Some(Value::Number(0))
            }
            Mnemonic::Cmp | Mnemonic::Test => return Ok(()),
            _ => None,
        };
        let value = value.filter(|value| {
            matches!(value, Value::Number(_)) || is_pointer_register(instruction.op0_register())
        });
        if let Some(value) = value {
            self.registers.insert(destination, value);
        } else {
            self.registers.remove(&destination);
        }
        Ok(())
    }
}

fn is_pointer_register(register: Register) -> bool {
    matches!(
        register,
        Register::RAX
            | Register::RBX
            | Register::RCX
            | Register::RDX
            | Register::RSI
            | Register::RDI
            | Register::RSP
            | Register::RBP
            | Register::R8
            | Register::R9
            | Register::R10
            | Register::R11
            | Register::R12
            | Register::R13
            | Register::R14
            | Register::R15
    )
}

fn register(value: Register) -> Register {
    use Register::*;
    match value {
        AL | AH | AX | EAX => RAX,
        BL | BH | BX | EBX => RBX,
        CL | CH | CX | ECX => RCX,
        DL | DH | DX | EDX => RDX,
        SIL | SI | ESI => RSI,
        DIL | DI | EDI => RDI,
        SPL | SP | ESP => RSP,
        BPL | BP | EBP => RBP,
        R8L | R8W | R8D => R8,
        R9L | R9W | R9D => R9,
        R10L | R10W | R10D => R10,
        R11L | R11W | R11D => R11,
        R12L | R12W | R12D => R12,
        R13L | R13W | R13D => R13,
        R14L | R14W | R14D => R14,
        R15L | R15W | R15D => R15,
        _ => value,
    }
}

#[derive(Clone, Copy, Default)]
struct Summary {
    receiver: Option<u32>,
    returned: Option<(u32, bool)>,
}

struct Factories<'a> {
    image: &'a PeImage,
    cache: BTreeMap<u32, Summary>,
    visiting: BTreeSet<u32>,
}

impl<'a> Factories<'a> {
    fn new(image: &'a PeImage) -> Self {
        Self {
            image,
            cache: BTreeMap::new(),
            visiting: BTreeSet::new(),
        }
    }

    fn object(&mut self, entry: u32) -> io::Result<u32> {
        let factory = code_pointer(self.image, checked_add(entry, ENTRY_FACTORY)?)?;
        let summary = self.summary(self.image.va_to_rva(factory)?)?;
        let (vtable, _) = summary
            .returned
            .ok_or_else(|| invalid("registry factory has no returned object identity"))?;
        class_name(self.image, vtable)?;
        Ok(vtable)
    }

    fn summary(&mut self, function: u32) -> io::Result<Summary> {
        if let Some(summary) = self.cache.get(&function) {
            return Ok(*summary);
        }
        if !self.visiting.insert(function) {
            return Err(invalid("registry constructor graph contains a cycle"));
        }
        let result = self.read_summary(function);
        self.visiting.remove(&function);
        if let Ok(summary) = result {
            self.cache.insert(function, summary);
        }
        result
    }

    fn read_summary(&mut self, function: u32) -> io::Result<Summary> {
        let instructions = function_body(self.image, function)?;
        if instructions.is_empty() {
            return Err(invalid("registry function has no instructions"));
        }
        let boundaries: BTreeMap<_, _> = instructions
            .iter()
            .enumerate()
            .map(|(index, instruction)| (instruction.ip(), index))
            .collect();
        let mut pending = vec![(0, State::new(), None)];
        let mut visited = BTreeSet::new();
        let mut result = Summary::default();
        while let Some((mut index, mut state, mut previous)) = pending.pop() {
            while let Some(instruction) = instructions.get(index) {
                if !visited.insert((index, state.clone(), previous)) {
                    break;
                }
                let mut next = index + 1;
                if instruction.flow_control() == FlowControl::ConditionalBranch {
                    let target = boundaries
                        .get(&instruction.near_branch_target())
                        .copied()
                        .ok_or_else(|| {
                            invalid("registry conditional branch leaves its function extent")
                        })?;
                    match branch_taken(
                        &state,
                        instruction,
                        previous.map(|index| &instructions[index]),
                    ) {
                        Some(true) => next = target,
                        Some(false) => {}
                        None => pending.push((target, state.clone(), None)),
                    }
                } else if let Some(target) = direct_target(self.image, instruction)? {
                    if instruction.mnemonic() == Mnemonic::Jmp
                        && let Some(&target) = boundaries.get(&instruction.near_branch_target())
                    {
                        previous = None;
                        index = target;
                        continue;
                    }
                    let allocation_root = self.image.va_to_rva(instruction.ip())?;
                    let receiver = state.registers.get(&Register::RCX).copied();
                    let summary = if target != OBJECT_ALLOCATION
                        && matches!(receiver, Some(Value::Object(_, 0)))
                    {
                        Some(self.summary(target)?)
                    } else {
                        None
                    };
                    state.clobber();
                    if target == OBJECT_ALLOCATION {
                        state.allocate(allocation_root);
                    } else if let Some(summary) = summary
                        && let Some(Value::Object(root, offset)) = receiver
                    {
                        if let Some(vtable) = summary.receiver {
                            state.tables.insert((root, offset), vtable);
                        }
                        if let Some((vtable, returned_receiver)) = summary.returned {
                            if returned_receiver {
                                state
                                    .registers
                                    .insert(Register::RAX, Value::Object(root, offset));
                            } else {
                                state.allocate(allocation_root);
                                if let Some(Value::Object(root, offset)) =
                                    state.registers.get(&Register::RAX).copied()
                                {
                                    state.tables.insert((root, offset), vtable);
                                }
                            }
                        }
                    }
                    if instruction.mnemonic() == Mnemonic::Jmp {
                        record_return(&state, &mut result)?;
                        break;
                    }
                } else if instruction.mnemonic() == Mnemonic::Call {
                    state.clobber();
                } else if instruction.mnemonic() == Mnemonic::Ret {
                    record_return(&state, &mut result)?;
                    break;
                } else {
                    state.simple(self.image, instruction)?;
                }
                previous = Some(index);
                index = next;
            }
        }
        Ok(result)
    }
}

fn branch_taken(
    state: &State,
    branch: &Instruction,
    previous: Option<&Instruction>,
) -> Option<bool> {
    if !matches!(branch.mnemonic(), Mnemonic::Je | Mnemonic::Jne) {
        return None;
    }
    let previous = previous?;
    if !matches!(previous.mnemonic(), Mnemonic::Test | Mnemonic::Cmp) {
        return None;
    }
    let left = state.operand(previous, 0)?;
    let right = state.operand(previous, 1)?;
    let zero = match (previous.mnemonic(), left, right) {
        (Mnemonic::Test, Value::Number(left), Value::Number(right)) => left & right == 0,
        (Mnemonic::Cmp, Value::Number(left), Value::Number(right)) => left == right,
        (Mnemonic::Test, Value::Object(root, 0), Value::Object(other, 0))
            if root == other
                && is_pointer_register(previous.op0_register())
                && is_pointer_register(previous.op1_register()) =>
        {
            false
        }
        (Mnemonic::Cmp, Value::Object(_, 0), Value::Number(0))
            if is_pointer_register(previous.op0_register()) =>
        {
            false
        }
        _ => return None,
    };
    Some(if branch.mnemonic() == Mnemonic::Je {
        zero
    } else {
        !zero
    })
}

fn record_return(state: &State, summary: &mut Summary) -> io::Result<()> {
    if let Some(vtable) = state.tables.get(&(0, 0)).copied() {
        if summary.receiver.is_some_and(|previous| previous != vtable) {
            return Err(invalid(
                "registry constructor leaves ambiguous receiver identities",
            ));
        }
        summary.receiver = Some(vtable);
    }
    if let Some(Value::Object(root, offset)) = state.registers.get(&Register::RAX).copied()
        && let Some(vtable) = state.tables.get(&(root, offset)).copied()
    {
        let returned = (vtable, root == 0 && offset == 0);
        if summary
            .returned
            .is_some_and(|previous| previous != returned)
        {
            return Err(invalid(
                "registry factory returns ambiguous object identities",
            ));
        }
        summary.returned = Some(returned);
    }
    Ok(())
}

fn function_body(image: &PeImage, start: u32) -> io::Result<Vec<Instruction>> {
    if let Ok((begin, end)) = image.function(start) {
        if begin != start {
            return Err(invalid("registry function pointer is not a function entry"));
        }
        return image.instructions(begin, end);
    }
    let mut end = image
        .executable_ranges()
        .iter()
        .find_map(|&(begin, end)| (begin <= start && start < end).then_some(end))
        .ok_or_else(|| invalid("registry function is outside executable sections"))?;
    if let Some(&(next, _)) = image.functions().iter().find(|&&(begin, _)| begin > start) {
        end = end.min(next);
    }
    let bytes = image.bytes(start, (end - start) as usize)?;
    let mut decoder = Decoder::with_ip(
        64,
        bytes,
        image.base + u64::from(start),
        DecoderOptions::NONE,
    );
    let mut result = Vec::new();
    while decoder.can_decode() {
        let instruction = decoder.decode();
        if instruction.is_invalid() {
            return Err(invalid("invalid instruction in registry leaf function"));
        }
        if matches!(
            instruction.op0_kind(),
            OpKind::NearBranch64 | OpKind::NearBranch32
        ) && !matches!(instruction.mnemonic(), Mnemonic::Call | Mnemonic::Jmp)
        {
            return Err(invalid(
                "registry leaf function has an unbounded conditional branch",
            ));
        }
        let terminal = instruction.mnemonic() == Mnemonic::Ret
            || (instruction.mnemonic() == Mnemonic::Jmp
                && direct_target(image, &instruction)?.is_some());
        result.push(instruction);
        if terminal {
            return Ok(result);
        }
    }
    Err(invalid("registry leaf function has no bounded terminator"))
}

fn constant_return(image: &PeImage, function: u32) -> io::Result<u32> {
    let mut result = None;
    for instruction in function_body(image, function)? {
        match instruction.mnemonic() {
            Mnemonic::Mov
                if instruction.op0_kind() == OpKind::Register
                    && instruction.op0_register() == Register::EAX
                    && instruction.op1_kind() == OpKind::Immediate32 =>
            {
                result = Some(instruction.immediate32());
            }
            Mnemonic::Xor
                if instruction.op0_register() == Register::EAX
                    && instruction.op1_kind() == OpKind::Register
                    && instruction.op1_register() == Register::EAX =>
            {
                result = Some(0)
            }
            Mnemonic::Ret => {
                return result.ok_or_else(|| invalid("trigger scope mask has no constant result"));
            }
            Mnemonic::Nop | Mnemonic::Int3 => {}
            _ => return Err(invalid("trigger scope mask has an unsupported getter")),
        }
    }
    Err(invalid("trigger scope mask has no return"))
}

pub(crate) fn class_name(image: &PeImage, vtable: u32) -> io::Result<String> {
    let locator_pointer = vtable
        .checked_sub(8)
        .ok_or_else(|| invalid("class identity address underflow"))?;
    let locator = image.va_to_rva(image.u64(locator_pointer)?)?;
    image.bytes(locator, 24)?;
    if image.u32(locator)? != 1 || image.u32(checked_add(locator, 20)?)? != locator {
        return Err(invalid("class identity locator is inconsistent"));
    }
    let descriptor = image.u32(checked_add(locator, 12)?)?;
    image.bytes(descriptor, 16)?;
    let hierarchy = image.u32(checked_add(locator, 16)?)?;
    image.bytes(hierarchy, 16)?;
    let decorated = image.string(checked_add(descriptor, 16)?)?;
    project_class_name(decorated)
}

fn project_class_name(decorated: &str) -> io::Result<String> {
    let raw = decorated
        .strip_prefix(".?AV")
        .or_else(|| decorated.strip_prefix(".?AU"))
        .and_then(|name| name.strip_suffix("@@"))
        .filter(|name| {
            !name.is_empty() && name.is_ascii() && !name.bytes().any(|byte| byte.is_ascii_control())
        })
        .ok_or_else(|| invalid("unsupported class identity decoration"))?;
    if let Some(template) = raw.strip_prefix("?$") {
        let (name, _) = template
            .split_once('@')
            .ok_or_else(|| invalid("invalid template class identity"))?;
        if !identifier(name) {
            return Err(invalid("invalid template class identity"));
        }
        return Ok(format!("{name}<>"));
    }
    let parts: Vec<_> = raw.split('@').collect();
    if parts.iter().all(|part| identifier(part)) {
        Ok(parts.into_iter().rev().collect::<Vec<_>>().join("::"))
    } else {
        Ok(raw.to_owned())
    }
}

fn identifier(value: &str) -> bool {
    let mut bytes = value.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;

    const CODE_START: u32 = 0x1000;
    const FACTORY_START: u32 = 0x1100;
    const CONSTRUCTOR_START: u32 = 0x1200;
    const ENTRY_TABLE: u32 = 0x3100;
    const OBJECT_TABLE: u32 = 0x3208;
    const HELPER_TABLE: u32 = 0x3308;

    fn write_u16(bytes: &mut [u8], at: usize, value: u16) {
        bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
    }

    fn write_u32(bytes: &mut [u8], at: usize, value: u32) {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn write_u64(bytes: &mut [u8], at: usize, value: u64) {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn offset(rva: u32) -> usize {
        if rva < 0x3000 {
            (rva - 0x1000 + 0x200) as usize
        } else {
            (rva - 0x3000 + 0x1200) as usize
        }
    }

    fn fixture_bytes() -> Vec<u8> {
        let mut bytes = vec![0; 0x2200];
        bytes[..2].copy_from_slice(b"MZ");
        write_u32(&mut bytes, 0x3c, 0x80);
        bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
        write_u16(&mut bytes, 0x84, 0x8664);
        write_u16(&mut bytes, 0x86, 2);
        write_u16(&mut bytes, 0x94, 240);
        write_u16(&mut bytes, 0x98, 0x20b);
        write_u64(&mut bytes, 0xb0, super::super::pe::PREFERRED_IMAGE_BASE);
        write_u32(&mut bytes, 0xb8, 0x1000);
        write_u32(&mut bytes, 0xbc, 0x200);
        write_u32(&mut bytes, 0xd0, 0x0260_0000);
        write_u32(&mut bytes, 0xd4, 0x200);
        write_u32(&mut bytes, 0x104, 16);
        write_u32(&mut bytes, 0x120, 0x3700);
        write_u32(&mut bytes, 0x124, 12);
        for (section, rva, raw, flags) in [
            (0x188, 0x1000, 0x200, 0x6000_0020),
            (0x1b0, 0x3000, 0x1200, 0x4000_0040),
        ] {
            write_u32(&mut bytes, section + 8, 0x1000);
            write_u32(&mut bytes, section + 12, rva);
            write_u32(&mut bytes, section + 16, 0x1000);
            write_u32(&mut bytes, section + 20, raw);
            write_u32(&mut bytes, section + 36, flags);
        }
        bytes[offset(CODE_START)] = 0xc3;
        write_u32(&mut bytes, offset(0x3700), CODE_START);
        write_u32(&mut bytes, offset(0x3704), CODE_START + 1);
        write_u32(&mut bytes, offset(0x3708), 0x3800);
        let base = super::super::pe::PREFERRED_IMAGE_BASE;
        write_u64(
            &mut bytes,
            offset(ENTRY_TABLE),
            base + u64::from(CODE_START),
        );
        write_u64(
            &mut bytes,
            offset(ENTRY_TABLE + ENTRY_FACTORY),
            base + u64::from(FACTORY_START),
        );
        for (vtable, locator, descriptor, name) in [
            (OBJECT_TABLE, 0x3400, 0x3500, ".?AVSyntheticRoot@@"),
            (HELPER_TABLE, 0x3420, 0x3540, ".?AVSyntheticHelper@@"),
        ] {
            write_u64(&mut bytes, offset(vtable - 8), base + u64::from(locator));
            write_u64(&mut bytes, offset(vtable), base + u64::from(CODE_START));
            write_u64(&mut bytes, offset(vtable + 8), base + u64::from(CODE_START));
            write_u32(&mut bytes, offset(locator), 1);
            write_u32(&mut bytes, offset(locator + 12), descriptor);
            write_u32(&mut bytes, offset(locator + 16), 0x3600);
            write_u32(&mut bytes, offset(locator + 20), locator);
            let at = offset(descriptor + 16);
            bytes[at..at + name.len()].copy_from_slice(name.as_bytes());
        }
        bytes
    }

    fn relative(code: &mut Vec<u8>, start: u32, opcode: &[u8], target: u32) {
        code.extend_from_slice(opcode);
        let next = start + code.len() as u32 + 4;
        let displacement = i32::try_from(i64::from(target) - i64::from(next)).unwrap();
        code.extend_from_slice(&displacement.to_le_bytes());
    }

    fn install_code(bytes: &mut [u8], start: u32, code: &[u8]) {
        let at = offset(start);
        bytes[at..at + code.len()].copy_from_slice(code);
    }

    fn allocation(code: &mut Vec<u8>) {
        code.extend_from_slice(&[0xb9, 0x40, 0, 0, 0]);
        relative(code, FACTORY_START, &[0xe8], OBJECT_ALLOCATION);
        code.extend_from_slice(&[0x48, 0x89, 0xc3]);
    }

    fn registration_fixture(clobber_arguments: bool) -> PeImage {
        let mut bytes = fixture_bytes();
        let mut code = Vec::new();
        allocation(&mut code);
        relative(&mut code, FACTORY_START, &[0x48, 0x8d, 0x0d], ENTRY_TABLE);
        code.extend_from_slice(&[0x48, 0x89, 0x08, 0x49, 0x89, 0xc0, 0xba, 42, 0, 0, 0]);
        if clobber_arguments {
            relative(&mut code, FACTORY_START, &[0xe8], CODE_START);
            code.extend_from_slice(&[0x49, 0x89, 0xd8]);
        }
        relative(&mut code, FACTORY_START, &[0xe9], TRIGGER_REGISTRATION);
        install_code(&mut bytes, FACTORY_START, &code);
        write_u32(&mut bytes, offset(0x3700), FACTORY_START);
        write_u32(
            &mut bytes,
            offset(0x3704),
            FACTORY_START + code.len() as u32,
        );
        PeImage::parse(bytes).unwrap()
    }

    fn trigger_registrations(image: &PeImage) -> io::Result<BTreeMap<u32, u32>> {
        let [sites] = call_sites(image, [TRIGGER_REGISTRATION])?;
        registrations(image, TRIGGER_REGISTRATION, sites, "synthetic")
    }

    #[test]
    fn registration_requires_the_entry_object_and_current_arguments() {
        let image = registration_fixture(false);
        assert_eq!(
            trigger_registrations(&image).unwrap(),
            BTreeMap::from([(42, ENTRY_TABLE)])
        );
        let image = registration_fixture(true);
        let error = trigger_registrations(&image).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("1 unsupported registration occurrences")
        );
    }

    #[test]
    fn one_pass_finds_each_targets_calls_and_jumps() {
        // The fixture's factory: a 5-byte size, the allocation call, a 3-byte
        // move, a 7-byte table address, 11 argument bytes, the helper call, a
        // 3-byte move and the registration jump.
        let image = registration_fixture(true);
        let [allocations, helpers, registrations, unused] = call_sites(
            &image,
            [
                OBJECT_ALLOCATION,
                CODE_START,
                TRIGGER_REGISTRATION,
                EFFECT_REGISTRATION,
            ],
        )
        .unwrap();
        assert_eq!(allocations, BTreeSet::from([FACTORY_START + 5]));
        assert_eq!(helpers, BTreeSet::from([FACTORY_START + 31]));
        assert_eq!(registrations, BTreeSet::from([FACTORY_START + 39]));
        assert!(unused.is_empty());
    }

    #[test]
    fn factory_tracks_the_returned_object_past_an_embedded_helper() {
        let mut bytes = fixture_bytes();
        let mut code = Vec::new();
        allocation(&mut code);
        relative(&mut code, FACTORY_START, &[0x48, 0x8d, 0x0d], OBJECT_TABLE);
        code.extend_from_slice(&[0x48, 0x89, 0x0b, 0x48, 0x8d, 0x53, 0x10]);
        relative(&mut code, FACTORY_START, &[0x48, 0x8d, 0x05], HELPER_TABLE);
        code.extend_from_slice(&[0x48, 0x89, 0x02, 0x48, 0x89, 0xd8, 0xc3]);
        install_code(&mut bytes, FACTORY_START, &code);
        let image = PeImage::parse(bytes).unwrap();
        assert_eq!(
            Factories::new(&image).object(ENTRY_TABLE).unwrap(),
            OBJECT_TABLE
        );
    }

    #[test]
    fn constructor_calls_preserve_the_receiver_and_derived_override() {
        let mut bytes = fixture_bytes();
        let mut code = Vec::new();
        allocation(&mut code);
        code.extend_from_slice(&[0x48, 0x89, 0xc1]);
        relative(&mut code, FACTORY_START, &[0xe8], CONSTRUCTOR_START);
        relative(&mut code, FACTORY_START, &[0x48, 0x8d, 0x05], OBJECT_TABLE);
        code.extend_from_slice(&[0x48, 0x89, 0x03, 0x48, 0x89, 0xd8, 0xc3]);
        install_code(&mut bytes, FACTORY_START, &code);
        let mut constructor = Vec::new();
        relative(
            &mut constructor,
            CONSTRUCTOR_START,
            &[0x48, 0x8d, 0x05],
            HELPER_TABLE,
        );
        constructor.extend_from_slice(&[0x48, 0x89, 0x01, 0x48, 0x89, 0xc8, 0xc3]);
        install_code(&mut bytes, CONSTRUCTOR_START, &constructor);
        let image = PeImage::parse(bytes).unwrap();
        assert_eq!(
            Factories::new(&image).object(ENTRY_TABLE).unwrap(),
            OBJECT_TABLE
        );
    }

    #[test]
    fn factory_keeps_the_successful_return_separate_from_allocation_failure() {
        let mut bytes = fixture_bytes();
        let mut code = Vec::new();
        allocation(&mut code);
        code.extend_from_slice(&[0x48, 0x85, 0xc0]);
        let failure_branch = code.len();
        relative(&mut code, FACTORY_START, &[0x0f, 0x84], FACTORY_START);
        relative(&mut code, FACTORY_START, &[0x48, 0x8d, 0x0d], OBJECT_TABLE);
        code.extend_from_slice(&[0x48, 0x89, 0x0b]);
        let success_branch = code.len();
        relative(&mut code, FACTORY_START, &[0xe9], FACTORY_START);
        let failure = code.len();
        code.extend_from_slice(&[0x31, 0xdb]);
        let finish = code.len();
        code.extend_from_slice(&[0x48, 0x89, 0xd8, 0xc3]);
        let failure_displacement = i32::try_from(failure - (failure_branch + 6)).unwrap();
        code[failure_branch + 2..failure_branch + 6]
            .copy_from_slice(&failure_displacement.to_le_bytes());
        let success_displacement = i32::try_from(finish - (success_branch + 5)).unwrap();
        code[success_branch + 1..success_branch + 5]
            .copy_from_slice(&success_displacement.to_le_bytes());
        install_code(&mut bytes, FACTORY_START, &code);
        write_u32(&mut bytes, offset(0x3700), FACTORY_START);
        write_u32(
            &mut bytes,
            offset(0x3704),
            FACTORY_START + code.len() as u32,
        );
        let image = PeImage::parse(bytes).unwrap();
        assert_eq!(
            Factories::new(&image).object(ENTRY_TABLE).unwrap(),
            OBJECT_TABLE
        );
    }

    #[test]
    fn factory_refuses_different_returned_identities_on_unknown_branches() {
        let mut bytes = fixture_bytes();
        let mut code = Vec::new();
        allocation(&mut code);
        // This input is not established by the factory, so both paths must be read.
        code.extend_from_slice(&[0x85, 0xd2]);
        let alternate_branch = code.len();
        relative(&mut code, FACTORY_START, &[0x0f, 0x84], FACTORY_START);
        relative(&mut code, FACTORY_START, &[0x48, 0x8d, 0x0d], OBJECT_TABLE);
        code.extend_from_slice(&[0x48, 0x89, 0x0b, 0x48, 0x89, 0xd8, 0xc3]);
        let alternate = code.len();
        relative(&mut code, FACTORY_START, &[0x48, 0x8d, 0x0d], HELPER_TABLE);
        code.extend_from_slice(&[0x48, 0x89, 0x0b, 0x48, 0x89, 0xd8, 0xc3]);
        let displacement = i32::try_from(alternate - (alternate_branch + 6)).unwrap();
        code[alternate_branch + 2..alternate_branch + 6]
            .copy_from_slice(&displacement.to_le_bytes());
        install_code(&mut bytes, FACTORY_START, &code);
        write_u32(&mut bytes, offset(0x3700), FACTORY_START);
        write_u32(
            &mut bytes,
            offset(0x3704),
            FACTORY_START + code.len() as u32,
        );
        let image = PeImage::parse(bytes).unwrap();
        let error = Factories::new(&image).object(ENTRY_TABLE).unwrap_err();
        assert!(error.to_string().contains("ambiguous object identities"));
    }

    #[test]
    fn constructor_cycles_and_inconsistent_class_locators_are_refused() {
        let mut bytes = fixture_bytes();
        let mut code = Vec::new();
        relative(&mut code, FACTORY_START, &[0xe9], FACTORY_START);
        install_code(&mut bytes, FACTORY_START, &code);
        let image = PeImage::parse(bytes).unwrap();
        assert!(Factories::new(&image).object(ENTRY_TABLE).is_err());
        let mut bytes = fixture_bytes();
        write_u32(&mut bytes, offset(0x3400 + 20), 0x3420);
        let image = PeImage::parse(bytes).unwrap();
        assert!(class_name(&image, OBJECT_TABLE).is_err());
    }

    #[test]
    fn reader_scope_mask_requires_a_constant_return() {
        let mut bytes = fixture_bytes();
        install_code(&mut bytes, FACTORY_START, &[0xb8, 0x24, 0, 0, 0, 0xc3]);
        let image = PeImage::parse(bytes).unwrap();
        assert_eq!(constant_return(&image, FACTORY_START).unwrap(), 0x24);
        let mut bytes = fixture_bytes();
        install_code(&mut bytes, FACTORY_START, &[0x83, 0xf8, 0, 0xc3]);
        let image = PeImage::parse(bytes).unwrap();
        assert!(constant_return(&image, FACTORY_START).is_err());
    }

    #[test]
    fn class_name_projection_keeps_specializations_and_namespaces() {
        assert_eq!(
            project_class_name(".?AVSynthetic@tools@@").unwrap(),
            "tools::Synthetic"
        );
        assert_eq!(
            project_class_name(".?AU?$Synthetic@H@@").unwrap(),
            "Synthetic<>"
        );
        assert_eq!(
            project_class_name(".?AVSynthetic@?A123@@").unwrap(),
            "Synthetic@?A123"
        );
        assert!(project_class_name("Synthetic").is_err());
    }

    fn installed() -> &'static RegistryTables {
        super::super::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must select the supported installation");
        &super::super::get().registries
    }

    #[test]
    #[ignore = "requires BATTLE_PLAN_GAME_DIR"]
    fn installed_trigger_registry_digest() {
        verify_names("triggers", &installed().triggers, 596, TRIGGERS_DIGEST).unwrap();
    }

    #[test]
    #[ignore = "requires BATTLE_PLAN_GAME_DIR"]
    fn installed_effect_registry_digest() {
        verify_names("effects", &installed().effects, 553, EFFECTS_DIGEST).unwrap();
    }

    #[test]
    #[ignore = "requires BATTLE_PLAN_GAME_DIR"]
    fn installed_trigger_class_digest() {
        installed().verify().unwrap();
    }

    #[test]
    #[ignore = "requires BATTLE_PLAN_GAME_DIR"]
    fn installed_effect_reader_digest() {
        installed().verify().unwrap();
    }

    #[test]
    #[ignore = "requires BATTLE_PLAN_GAME_DIR"]
    fn installed_effect_identity_digest() {
        installed().verify().unwrap();
    }
}
