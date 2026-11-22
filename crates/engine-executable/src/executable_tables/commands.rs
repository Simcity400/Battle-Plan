use super::{pe::PeImage, registries::class_name};
use iced_x86::{Decoder, DecoderOptions, FlowControl, Instruction, Mnemonic, OpKind, Register};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    io,
};

mod owners;
pub use owners::{Object, Source};

const COMMAND_TYPE: u64 = 0xfa4c_d01a_a014_fc8b;
const ACCESS_CHECK_TEXT: (usize, u64) = (12, 0x2483_988f_1cc5_66b3);
const CONFIRMATION_BUDGET: usize = 4096;
const LEAF_BYTES: u32 = 128;

#[derive(Clone, Copy, Debug)]
pub enum Target {
    Unknown,
    Country(u32),
    Group(u32),
    Unit(u32),
    Units(u32),
    Wings(u32),
    Sources(&'static [Source]),
}

#[derive(Clone, Copy, Debug)]
pub enum Token {
    Constant(u32),
    Word(u32),
}

#[derive(Debug)]
pub struct Kind {
    pub vtable: u32,
    pub name: String,
    pub token: Token,
    pub target: Target,
}

#[derive(Debug)]
pub struct CommandTables {
    pub kinds: Vec<Kind>,
    pub(super) objects: Vec<(u32, u32)>,
}

impl CommandTables {
    pub(super) fn recorded(kinds: Vec<Kind>, objects: Vec<(u32, u32)>) -> Self {
        Self { kinds, objects }
    }

    pub fn content_digest(&self) -> [u8; 32] {
        let mut kinds: Vec<_> = self.kinds.iter().collect();
        kinds.sort_unstable_by(|left, right| left.name.cmp(&right.name));
        let mut digest = Sha256::new();
        for kind in kinds {
            digest.update((kind.name.len() as u32).to_le_bytes());
            digest.update(kind.name.as_bytes());
            match kind.token {
                Token::Constant(value) => {
                    digest.update([0]);
                    digest.update(value.to_le_bytes());
                }
                Token::Word(_) => digest.update([1]),
            }
            target_content(kind.target, &mut digest);
        }
        let mut masks: Vec<_> = self.objects.iter().map(|&(_, mask)| mask).collect();
        masks.sort_unstable();
        for mask in masks {
            digest.update(mask.to_le_bytes());
        }
        digest.finalize().into()
    }

    pub(super) fn decode(image: &PeImage) -> io::Result<Self> {
        let mut bases = Vec::new();
        let mut commands = Vec::new();
        let mut families = Vec::new();
        let mut names = HashMap::new();
        let mut primaries: Vec<_> = image
            .classes()
            .iter()
            .filter(|class| class.offset == 0)
            .map(|class| (class.locator, class.descriptor))
            .collect();
        primaries.dedup();
        for (locator, own) in primaries {
            let mut mask = 0;
            let mut command = false;
            for (descriptor, displacement) in image.locator_bases(locator)? {
                if displacement != 0 {
                    continue;
                }
                let class = type_hash(image, &mut names, descriptor)?;
                if let Some(object) = owners::family(class) {
                    mask |= object.mask();
                }
                if class == COMMAND_TYPE {
                    if descriptor == own {
                        bases.push(locator);
                    } else {
                        command = true;
                    }
                }
            }
            if command {
                commands.push(locator);
            }
            if mask != 0 {
                families.push((locator, mask));
            }
        }
        let [base] = bases[..] else {
            return Err(invalid("command base type missing or repeated"));
        };
        let mut wanted: Vec<_> = commands
            .iter()
            .chain(&[base])
            .copied()
            .chain(families.iter().map(|&(locator, _)| locator))
            .collect();
        wanted.sort_unstable();
        wanted.dedup();
        let vtables: Vec<(u32, u32)> = image
            .classes()
            .iter()
            .filter(|class| wanted.binary_search(&class.locator).is_ok())
            .map(|class| (class.locator, class.vtable))
            .collect();
        let vtable_of = |locator: u32| {
            let mut found = vtables
                .iter()
                .filter(|&&(owner, _)| owner == locator)
                .map(|&(_, vtable)| vtable);
            match (found.next(), found.next()) {
                (Some(vtable), None) => Ok(vtable),
                _ => Err(invalid("command type vtable missing or repeated")),
            }
        };
        let base = slots(image, vtable_of(base)?)?;
        let mut concrete = Vec::with_capacity(commands.len());
        for &locator in &commands {
            concrete.push(vtable_of(locator)?);
        }
        let type_slot = unique_slot(&base, "type", |index| {
            let mut issued = false;
            for &vtable in &concrete {
                let function = slot(image, vtable, index)?;
                if function != base[index] {
                    if token(image, function).is_none() {
                        return Ok(false);
                    }
                    issued = true;
                }
            }
            Ok(issued)
        })?;
        concrete.retain(|&vtable| {
            slot(image, vtable, type_slot).is_ok_and(|reader| reader != base[type_slot])
        });
        let check = access_check(image)?;
        let permission_slot = unique_slot(&base, "permission", |index| {
            if index == type_slot {
                return Ok(false);
            }
            let mut checked = HashSet::new();
            for &vtable in &concrete {
                let function = slot(image, vtable, index)?;
                if function != base[index]
                    && checked.insert(function)
                    && reaches(image, function, check)
                {
                    return Ok(true);
                }
            }
            Ok(false)
        })?;
        let mut confirmed = HashMap::new();
        let mut kinds = Vec::with_capacity(concrete.len());
        for vtable in concrete {
            let name = class_name(image, vtable)?;
            let token = token(image, slot(image, vtable, type_slot)?)
                .ok_or_else(|| invalid(format!("command type reader unsupported: {name}")))?;
            let target = owners::target(class_hash(&name));
            let fields = target_fields(target);
            if !fields.is_empty() {
                let read = confirmed
                    .entry(slot(image, vtable, permission_slot)?)
                    .or_insert_with_key(|&check| operand_offsets(image, check));
                if fields
                    .iter()
                    .any(|&field| read.binary_search(&i64::from(field)).is_err())
                {
                    return Err(invalid(format!(
                        "command owner operands unconfirmed: {name}"
                    )));
                }
            }
            kinds.push(Kind {
                vtable,
                name,
                token,
                target,
            });
        }
        kinds.sort_unstable_by_key(|kind| kind.vtable);
        if kinds.is_empty()
            || kinds
                .windows(2)
                .any(|pair| pair[0].vtable == pair[1].vtable)
        {
            return Err(invalid("command type catalog missing or duplicated"));
        }
        let mut objects: Vec<_> = vtables
            .iter()
            .filter_map(|&(locator, vtable)| {
                families
                    .binary_search_by_key(&locator, |&(family, _)| family)
                    .ok()
                    .map(|index| (vtable, families[index].1))
            })
            .collect();
        objects.sort_unstable();
        objects.dedup();
        if objects.is_empty() {
            return Err(invalid("command operand identities missing"));
        }
        Ok(Self { kinds, objects })
    }

    pub fn object(&self, vtable: u32, expected: Object) -> bool {
        self.objects
            .binary_search_by_key(&vtable, |&(identity, _)| identity)
            .is_ok_and(|index| self.objects[index].1 & expected.mask() != 0)
    }
}

pub fn class_hash(name: &str) -> u64 {
    hash(name.as_bytes())
}

pub(super) fn type_hash(
    image: &PeImage,
    names: &mut HashMap<u32, u64>,
    descriptor: u32,
) -> io::Result<u64> {
    if let Some(&class) = names.get(&descriptor) {
        return Ok(class);
    }
    let class = hash(image.type_name(descriptor)?.as_bytes());
    names.insert(descriptor, class);
    Ok(class)
}

fn hash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3)
    })
}

fn target_fields(target: Target) -> Vec<u32> {
    match target {
        Target::Unknown => Vec::new(),
        Target::Country(field)
        | Target::Group(field)
        | Target::Unit(field)
        | Target::Units(field)
        | Target::Wings(field) => vec![field],
        Target::Sources(sources) => sources
            .iter()
            .flat_map(|source| source.fields())
            .flatten()
            .collect(),
    }
}

fn target_content(target: Target, digest: &mut Sha256) {
    let (tag, field) = match target {
        Target::Unknown => (0, 0),
        Target::Country(field) => (1, field),
        Target::Group(field) => (2, field),
        Target::Unit(field) => (3, field),
        Target::Units(field) => (4, field),
        Target::Wings(field) => (5, field),
        Target::Sources(sources) => {
            digest.update([6, sources.len() as u8]);
            for &source in sources {
                let (tag, object) = match source {
                    Source::Country(_) => (0, None),
                    Source::Reference(_, object) => (1, Some(object)),
                    Source::References(_, object) => (2, Some(object)),
                    Source::Pointer(_, object) => (3, Some(object)),
                    Source::Province(_) => (4, None),
                    Source::Building { .. } => (5, None),
                    Source::NavalSide { .. } => (6, None),
                    Source::ScriptCountry { .. } => (7, None),
                };
                digest.update([tag, object.map_or(u8::MAX, |object| object as u8)]);
                for field in source.fields().into_iter().flatten() {
                    digest.update(field.to_le_bytes());
                }
            }
            return;
        }
    };
    digest.update([tag]);
    digest.update(field.to_le_bytes());
}

fn unique_slot(
    base: &[u32],
    role: &str,
    mut matches: impl FnMut(usize) -> io::Result<bool>,
) -> io::Result<usize> {
    let mut found = None;
    for index in 0..base.len() {
        if matches(index)? && found.replace(index).is_some() {
            return Err(invalid(format!("command {role} slot is ambiguous")));
        }
    }
    found.ok_or_else(|| invalid(format!("command {role} slot missing")))
}

fn slots(image: &PeImage, vtable: u32) -> io::Result<Vec<u32>> {
    let mut slots = Vec::new();
    loop {
        let entry = image.u64(at(vtable, slots.len() as u32 * 8)?)?;
        if !image.is_code_pointer(entry) {
            break;
        }
        slots.push(image.va_to_rva(entry)?);
    }
    Ok(slots)
}

fn slot(image: &PeImage, vtable: u32, index: usize) -> io::Result<u32> {
    let entry = image.u64(at(vtable, index as u32 * 8)?)?;
    if !image.is_code_pointer(entry) {
        return Err(invalid("command vtable is shorter than its base"));
    }
    image.va_to_rva(entry)
}

fn token(image: &PeImage, function: u32) -> Option<Token> {
    let mut decoder = Decoder::with_ip(
        64,
        image.bytes(function, 16).ok()?,
        image.base + u64::from(function),
        DecoderOptions::NONE,
    );
    let load = decoder.decode();
    let constant = load.mnemonic() == Mnemonic::Mov
        && load.op0_kind() == OpKind::Memory
        && load.memory_base() == Register::RDX
        && load.memory_displacement64() == 0
        && load.op1_kind() == OpKind::Immediate32;
    if !constant
        && (load.mnemonic() != Mnemonic::Mov
            || load.op0_register() != Register::EAX
            || load.op1_kind() != OpKind::Memory
            || !load.is_ip_rel_memory_operand())
    {
        return None;
    }
    let token = if constant {
        Token::Constant(load.immediate32())
    } else {
        Token::Word(image.va_to_rva(load.ip_rel_memory_address()).ok()?)
    };
    let store = if constant { load } else { decoder.decode() };
    let result = decoder.decode();
    let end = decoder.decode();
    (store.mnemonic() == Mnemonic::Mov
        && store.memory_base() == Register::RDX
        && store.memory_displacement64() == 0
        && (constant || store.op1_register() == Register::EAX)
        && result.mnemonic() == Mnemonic::Mov
        && result.op0_register() == Register::RAX
        && result.op1_register() == Register::RDX
        && end.mnemonic() == Mnemonic::Ret)
        .then_some(token)
}

fn access_check(image: &PeImage) -> io::Result<u32> {
    match text_users(image, ACCESS_CHECK_TEXT)?[..] {
        [(start, _)] => Ok(start),
        [] => Err(invalid("command access check missing")),
        _ => Err(invalid("command access check repeated")),
    }
}

fn text_users(image: &PeImage, anchor: (usize, u64)) -> io::Result<Vec<(u32, u32)>> {
    image.referencing_functions(
        image
            .rip_leas()
            .iter()
            .filter(|lea| is_text(image, lea.target, anchor))
            .map(|lea| lea.instruction),
    )
}

pub(super) fn is_text(image: &PeImage, rva: u32, (length, text): (usize, u64)) -> bool {
    image
        .text_at(rva, length)
        .is_some_and(|bytes| hash(bytes) == text)
}

fn reaches(image: &PeImage, function: u32, target: u32) -> bool {
    let (end, leaf) = match image.function(function) {
        Ok((start, end)) if start == function => (end, false),
        _ => (function.saturating_add(LEAF_BYTES), true),
    };
    let Some(&(_, limit)) = image
        .executable_ranges()
        .iter()
        .find(|&&(start, end)| (start..end).contains(&function))
    else {
        return false;
    };
    let Ok(bytes) = image.bytes(function, (end.min(limit) - function) as usize) else {
        return false;
    };
    let target = image.base + u64::from(target);
    let mut decoder = Decoder::with_ip(
        64,
        bytes,
        image.base + u64::from(function),
        DecoderOptions::NONE,
    );
    while decoder.can_decode() {
        let instruction = decoder.decode();
        if instruction.is_invalid() {
            break;
        }
        if matches!(instruction.mnemonic(), Mnemonic::Call | Mnemonic::Jmp)
            && instruction.op0_kind() == OpKind::NearBranch64
            && instruction.near_branch_target() == target
        {
            return true;
        }
        if leaf
            && matches!(
                instruction.flow_control(),
                FlowControl::Return
                    | FlowControl::UnconditionalBranch
                    | FlowControl::IndirectBranch
                    | FlowControl::Interrupt
            )
        {
            break;
        }
    }
    false
}

fn decode(image: &PeImage, rva: u32) -> io::Result<Instruction> {
    let (_, end) = image
        .executable_ranges()
        .iter()
        .copied()
        .find(|&(start, end)| (start..end).contains(&rva))
        .ok_or_else(|| invalid("command code lies outside executable storage"))?;
    let bytes = image.bytes(rva, (end - rva).min(15) as usize)?;
    let instruction =
        Decoder::with_ip(64, bytes, image.base + u64::from(rva), DecoderOptions::NONE).decode();
    if instruction.is_invalid() {
        return Err(invalid("command code has an invalid instruction"));
    }
    Ok(instruction)
}

fn operand_offsets(image: &PeImage, function: u32) -> Vec<i64> {
    let mut found = Vec::new();
    let mut budget = CONFIRMATION_BUDGET;
    trace(image, function, 0, true, &mut budget, &mut found);
    found.sort_unstable();
    found.dedup();
    found
}

fn trace(
    image: &PeImage,
    start: u32,
    receiver: i64,
    follow: bool,
    budget: &mut usize,
    found: &mut Vec<i64>,
) {
    let mut initial = [None; 16];
    initial[Register::RCX.number()] = Some(receiver);
    let mut work = vec![(start, initial)];
    let mut seen = HashSet::new();
    while let Some((mut at, mut state)) = work.pop() {
        while *budget > 0 && seen.insert(at) {
            *budget -= 1;
            let Ok(instruction) = decode(image, at) else {
                break;
            };
            at += instruction.len() as u32;
            if let Some(offset) = memory_offset(&instruction, &state) {
                found.push(offset);
            }
            match instruction.flow_control() {
                FlowControl::Return | FlowControl::Interrupt | FlowControl::IndirectBranch => {
                    break;
                }
                FlowControl::UnconditionalBranch => {
                    let Ok(target) = branch(image, &instruction) else {
                        break;
                    };
                    at = target;
                    continue;
                }
                FlowControl::ConditionalBranch => {
                    if let Ok(target) = branch(image, &instruction) {
                        work.push((target, state));
                    }
                    continue;
                }
                FlowControl::Call | FlowControl::IndirectCall => {
                    if follow
                        && instruction.op0_kind() == OpKind::NearBranch64
                        && let Some(receiver) = state[Register::RCX.number()]
                        && let Ok(target) = branch(image, &instruction)
                    {
                        trace(image, target, receiver, false, budget, found);
                    }
                    for register in [
                        Register::RAX,
                        Register::RCX,
                        Register::RDX,
                        Register::R8,
                        Register::R9,
                        Register::R10,
                        Register::R11,
                    ] {
                        state[register.number()] = None;
                    }
                    continue;
                }
                _ => {}
            }
            track(&instruction, &mut state, found);
        }
    }
}

fn branch(image: &PeImage, instruction: &Instruction) -> io::Result<u32> {
    image.va_to_rva(instruction.near_branch_target())
}

fn memory_offset(instruction: &Instruction, state: &[Option<i64>; 16]) -> Option<i64> {
    if !(0..instruction.op_count()).any(|operand| instruction.op_kind(operand) == OpKind::Memory)
        || instruction.memory_index() != Register::None
        || !instruction.memory_base().is_gpr64()
    {
        return None;
    }
    Some(state[instruction.memory_base().number()]? + instruction.memory_displacement64() as i64)
}

fn track(instruction: &Instruction, state: &mut [Option<i64>; 16], found: &mut Vec<i64>) {
    if instruction.op0_kind() != OpKind::Register || !instruction.op0_register().is_gpr() {
        return;
    }
    let destination = instruction.op0_register();
    let full = destination.full_register().number();
    let whole = destination.is_gpr64();
    state[full] = match instruction.mnemonic() {
        Mnemonic::Cmp | Mnemonic::Test | Mnemonic::Push => return,
        Mnemonic::Mov
            if whole
                && instruction.op1_kind() == OpKind::Register
                && instruction.op1_register().is_gpr64() =>
        {
            state[instruction.op1_register().number()]
        }
        Mnemonic::Lea
            if whole
                && instruction.memory_index() == Register::None
                && instruction.memory_base().is_gpr64() =>
        {
            state[instruction.memory_base().number()]
                .map(|offset| offset + instruction.memory_displacement64() as i64)
        }
        Mnemonic::Add
            if whole
                && matches!(
                    instruction.op1_kind(),
                    OpKind::Immediate8to64 | OpKind::Immediate32to64
                ) =>
        {
            let offset = state[full].map(|offset| offset + instruction.immediate(1) as i64);
            found.extend(offset);
            offset
        }
        _ => None,
    };
}

fn at(rva: u32, offset: u32) -> io::Result<u32> {
    rva.checked_add(offset)
        .ok_or_else(|| invalid("command address overflows"))
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executable_tables::synthetic_image::{Code, RDATA, SyntheticImage, TEXT};

    const CONSTANT: u32 = TEXT;
    const WORD: u32 = TEXT + 0x20;
    const PLAIN: u32 = TEXT + 0x40;
    const CALLER: u32 = TEXT + 0x60;
    const TARGET: u32 = TEXT + 0x80;
    const CALLEE: u32 = TEXT + 0xa0;
    const LEAF: u32 = TEXT + 0x300;
    const STOPPED_LEAF: u32 = TEXT + 0x320;
    const VTABLE: u32 = RDATA + 0x100;
    const TOKEN_WORD: u32 = RDATA + 0x200;

    fn image() -> PeImage {
        let mut image = SyntheticImage::new();
        image
            .function(
                Code::at(CONSTANT)
                    .raw(&[0xc7, 0x02, 0x2a, 0, 0, 0])
                    .raw(&[0x48, 0x89, 0xd0])
                    .ret(),
            )
            .function(
                Code::at(WORD)
                    .rip(&[0x8b, 0x05], TOKEN_WORD)
                    .raw(&[0x89, 0x02])
                    .raw(&[0x48, 0x89, 0xd0])
                    .ret(),
            )
            .function(Code::at(PLAIN).ret())
            .function(
                Code::at(CALLER)
                    .raw(&[0x8b, 0x41, 0x40])
                    .raw(&[0x48, 0x8d, 0x49, 0x10])
                    .call(CALLEE)
                    .raw(&[0x8b, 0x41, 0x60])
                    .raw(&[0x48, 0x89, 0xc8])
                    .call(TARGET)
                    .ret(),
            )
            .function(Code::at(TARGET).ret())
            .function(Code::at(CALLEE).raw(&[0x8b, 0x41, 0x08]).ret())
            .pointer(VTABLE, CONSTANT)
            .pointer(VTABLE + 8, WORD)
            .pointer(VTABLE + 16, PLAIN)
            .put(VTABLE + 24, &0_u64.to_le_bytes());
        image
            .code(Code::at(LEAF).rip(&[0xe9], TARGET).ret())
            .code(Code::at(STOPPED_LEAF).ret().call(TARGET));
        image.image()
    }

    #[test]
    fn a_type_reader_stores_a_constant_or_a_word_and_returns_its_output() {
        let image = image();
        assert!(matches!(token(&image, CONSTANT), Some(Token::Constant(0x2a))));
        assert!(matches!(token(&image, WORD), Some(Token::Word(TOKEN_WORD))));
        assert!(token(&image, PLAIN).is_none());
        assert!(token(&image, CALLER).is_none());
    }

    #[test]
    fn a_vtable_holds_its_code_pointers_up_to_the_first_other_value() {
        let image = image();
        assert_eq!(slots(&image, VTABLE).unwrap(), [CONSTANT, WORD, PLAIN]);
        assert_eq!(slot(&image, VTABLE, 1).unwrap(), WORD);
        assert!(slot(&image, VTABLE, 3).is_err());
    }

    #[test]
    fn a_check_is_reached_by_a_whole_function_or_a_leaf_before_it_returns() {
        let image = image();
        assert!(reaches(&image, CALLER, TARGET));
        assert!(!reaches(&image, CALLEE, TARGET));
        assert!(reaches(&image, LEAF, TARGET));
        assert!(!reaches(&image, STOPPED_LEAF, TARGET));
    }

    #[test]
    fn operand_offsets_follow_the_receiver_into_one_call_and_forget_it_after() {
        let image = image();
        assert_eq!(operand_offsets(&image, CALLER), [0x10, 0x18, 0x40]);
    }

    #[test]
    fn a_role_takes_exactly_one_slot() {
        assert_eq!(unique_slot(&[0; 4], "type", |index| Ok(index == 2)).unwrap(), 2);
        let ambiguous = unique_slot(&[0; 4], "type", |index| Ok(index > 1)).unwrap_err();
        assert!(ambiguous.to_string().contains("type slot is ambiguous"));
        let missing = unique_slot(&[0; 4], "permission", |_| Ok(false)).unwrap_err();
        assert!(missing.to_string().contains("permission slot missing"));
    }
}
