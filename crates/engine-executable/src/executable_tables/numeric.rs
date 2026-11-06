use super::{
    commands::{is_text, type_hash},
    leak_string,
    pe::PeImage,
};
use iced_x86::{
    Decoder, DecoderOptions, FlowControl, Instruction, MemorySize, Mnemonic, OpKind, Register,
};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeSet, HashMap},
    io,
};

/// Length and FNV-1a of the text only the numeric property dispatcher reports.
const DISPATCHER_TEXT: (usize, u64) = (29, 0x962d_6ca5_bde6_e20d);
/// FNV-1a of the decorated callable type every numeric callback implements.
const CALLBACK_TYPE: u64 = 0xccac_759a_1467_1f98;
/// Length and FNV-1a of the name the executable documents each scope's properties under.
const COUNTRY_SCOPE: (usize, u64) = (7, 0x9cf7_402f_e920_f7d1);
const STATE_SCOPE: (usize, u64) = (5, 0xae23_9213_b7c7_0376);
const UNIT_LEADER_SCOPE: (usize, u64) = (10, 0x0699_b0f6_19e0_73d2);
const CHARACTER_SCOPE: (usize, u64) = (9, 0xabfe_23c4_5dfe_3050);
const PROPERTY_ROW_WIDTH: u32 = 16;
const LOOKUP_BYTES: u32 = 14;
const CALLBACK_VALUE_READER: u32 = 16;
const CALLBACK_INPUT_READER: u32 = 24;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum NumericScope {
    Country,
    State,
    UnitLeader,
    Character,
}

impl NumericScope {
    const ALL: [Self; 4] = [
        Self::Country,
        Self::State,
        Self::UnitLeader,
        Self::Character,
    ];

    fn documented(self) -> (usize, u64) {
        match self {
            Self::Country => COUNTRY_SCOPE,
            Self::State => STATE_SCOPE,
            Self::UnitLeader => UNIT_LEADER_SCOPE,
            Self::Character => CHARACTER_SCOPE,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NumericIdentity {
    Scope {
        scope: NumericScope,
        ordinal: u32,
        handler_rva: u32,
    },
    Callback {
        vtable_rva: u32,
        value_reader_rva: u32,
        input_reader_rva: u32,
    },
}

#[derive(Debug)]
pub struct NumericProperty {
    pub name: &'static str,
    pub identity: NumericIdentity,
}

#[derive(Debug)]
pub struct NumericTables {
    pub properties: Vec<NumericProperty>,
}

/// A registry the dispatcher searches: its row count and table, and the handlers it pairs with.
struct Registry {
    count: u32,
    table: u32,
    handlers: BTreeSet<u32>,
}

impl NumericTables {
    /// SHA-256 of this table's build-independent content, comparable across builds.
    pub fn content_digest(&self) -> [u8; 32] {
        let mut digest = Sha256::new();
        for property in &self.properties {
            match property.identity {
                NumericIdentity::Scope { scope, ordinal, .. } => {
                    digest.update([0, scope as u8]);
                    digest.update(ordinal.to_le_bytes());
                }
                NumericIdentity::Callback { .. } => digest.update([1]),
            }
            digest.update((property.name.len() as u32).to_le_bytes());
            digest.update(property.name.as_bytes());
        }
        digest.finalize().into()
    }

    pub fn decode(image: &PeImage) -> io::Result<Self> {
        let callbacks = callback_vtables(image)?;
        let mut anchors = Vec::new();
        let mut callback_references = Vec::new();
        let mut row_references = Vec::new();
        for lea in image.rip_leas() {
            if is_text(image, lea.target, DISPATCHER_TEXT) {
                anchors.push(lea.instruction);
            }
            if callbacks.binary_search(&lea.target).is_ok() {
                callback_references.push((lea.target, lea.instruction));
            }
            // Initializers pass each row's address in the second argument.
            if lea.register == Register::RDX.number() {
                row_references.push((lea.target, lea.instruction));
            }
        }
        let registries = registries(image, anchors)?;
        let mut scoped: [Option<(u32, Vec<(String, u32)>)>; 4] = Default::default();
        for registry in &registries {
            let initializer = match image.referencing_functions(
                row_references
                    .iter()
                    .filter(|&&(target, _)| target == registry.table)
                    .map(|&(_, at)| at),
            )?[..]
            {
                [(start, _)] => start,
                [] => return Err(invalid("numeric property initializer missing")),
                _ => return Err(invalid("numeric property initializer repeated")),
            };
            let (scope, rows) = decode_registry(image, registry, initializer)?;
            let Some(scope) = scope else {
                continue;
            };
            let handler = match registry.handlers.iter().collect::<Vec<_>>()[..] {
                [&handler] => handler,
                _ => return Err(invalid("numeric property handler missing or ambiguous")),
            };
            if scoped[scope as usize].replace((handler, rows)).is_some() {
                return Err(invalid("numeric property scope repeated"));
            }
        }
        let mut properties = Vec::new();
        for (scope, registry) in NumericScope::ALL.into_iter().zip(scoped) {
            let (handler_rva, rows) =
                registry.ok_or_else(|| invalid("numeric property scope missing"))?;
            properties.extend(rows.into_iter().map(|(name, ordinal)| NumericProperty {
                name: leak_string(name),
                identity: NumericIdentity::Scope {
                    scope,
                    ordinal,
                    handler_rva,
                },
            }));
        }
        // Each callback's own copy also names its vtable; only the initializer names them all.
        let mut common: Option<BTreeSet<(u32, u32)>> = None;
        for &vtable in &callbacks {
            let holders: BTreeSet<_> = callback_references
                .iter()
                .filter(|&&(target, _)| target == vtable)
                .filter_map(|&(_, at)| image.function(at).ok())
                .collect();
            common = Some(match common {
                None => holders,
                Some(common) => common.intersection(&holders).copied().collect(),
            });
        }
        let [(initializer, end)] = common.unwrap_or_default().into_iter().collect::<Vec<_>>()[..]
        else {
            return Err(invalid("numeric callback initializer missing or repeated"));
        };
        decode_callbacks(image, initializer, end, &callbacks, &mut properties)?;
        Ok(Self { properties })
    }

    /// Exact spelling and registry order are significant to numeric admission.
    pub fn exact(&self, name: &str) -> Option<&NumericProperty> {
        self.properties
            .iter()
            .find(|property| property.name == name)
    }
}

/// The registries the numeric property dispatcher searches, each with the handlers it calls on
/// the index that registry's search found.
fn registries(image: &PeImage, anchors: Vec<u32>) -> io::Result<Vec<Registry>> {
    let (start, end) = match image.referencing_functions(anchors)?[..] {
        [extent] => extent,
        [] => return Err(invalid("numeric property dispatcher missing")),
        _ => return Err(invalid("numeric property dispatcher repeated")),
    };
    let code = image.instructions(start, end)?;
    let mut registries = Vec::new();
    let mut slots = Vec::new();
    for (at, instruction) in code.iter().enumerate() {
        let Some(target) = direct_call(image, instruction) else {
            continue;
        };
        let Some((count, table)) = lookup(image, target) else {
            continue;
        };
        // The search stores the index it finds in a stack slot before the next call.
        let slot = code[at + 1..]
            .iter()
            .take_while(|instruction| instruction.mnemonic() != Mnemonic::Call)
            .find_map(|instruction| {
                (instruction.mnemonic() == Mnemonic::Mov
                    && instruction.op0_kind() == OpKind::Memory
                    && instruction.op1_kind() == OpKind::Register
                    && instruction.op1_register().is_gpr32())
                .then(|| stack_slot(instruction))
                .flatten()
            })
            .ok_or_else(|| invalid("numeric property search has no index slot"))?;
        if slots.contains(&slot) {
            return Err(invalid("numeric property searches share an index slot"));
        }
        slots.push(slot);
        registries.push(Registry {
            count,
            table,
            handlers: BTreeSet::new(),
        });
    }
    for (at, instruction) in code.iter().enumerate() {
        let Some(handler) = direct_call(image, instruction) else {
            continue;
        };
        let operands = code[at.saturating_sub(3)..at]
            .iter()
            .rev()
            .take_while(|instruction| instruction.flow_control() == FlowControl::Next);
        let mut index = None;
        let mut output = false;
        for operand in operands {
            if operand.mnemonic() == Mnemonic::Mov
                && operand.op0_kind() == OpKind::Register
                && operand.op0_register() == Register::R8D
            {
                index.get_or_insert(*operand);
            }
            output |= operand.mnemonic() == Mnemonic::Lea
                && operand.op0_register() == Register::RDX
                && stack_base(operand.memory_base());
        }
        let Some(index) = index.filter(|_| output) else {
            continue;
        };
        let slot = if index.op1_kind() == OpKind::Register {
            let source = index.op1_register();
            code[..at].iter().rev().find_map(|load| {
                (load.mnemonic() == Mnemonic::Mov
                    && load.op0_kind() == OpKind::Register
                    && load.op0_register() == source)
                    .then(|| stack_slot(load))
                    .flatten()
                    .filter(|slot| slots.contains(slot))
            })
        } else {
            stack_slot(&index)
        };
        if let Some(position) = slot.and_then(|slot| slots.iter().position(|&known| known == slot))
        {
            registries[position].handlers.insert(handler);
        }
    }
    Ok(registries)
}

fn direct_call(image: &PeImage, instruction: &Instruction) -> Option<u32> {
    (instruction.mnemonic() == Mnemonic::Call && instruction.op0_kind() == OpKind::NearBranch64)
        .then(|| image.va_to_rva(instruction.near_branch_target()).ok())
        .flatten()
}

/// A registry lookup: it stores the row count through its argument and returns the rows.
fn lookup(image: &PeImage, function: u32) -> Option<(u32, u32)> {
    let mut decoder = Decoder::with_ip(
        64,
        image.bytes(function, LOOKUP_BYTES as usize).ok()?,
        image.base + u64::from(function),
        DecoderOptions::NONE,
    );
    let (count, rows, end) = (decoder.decode(), decoder.decode(), decoder.decode());
    (count.mnemonic() == Mnemonic::Mov
        && receiver_memory(&count, Register::RCX)
        && count.op1_kind() == OpKind::Immediate32
        && rows.mnemonic() == Mnemonic::Lea
        && rows.op0_register() == Register::RAX
        && rows.is_ip_rel_memory_operand()
        && end.mnemonic() == Mnemonic::Ret)
        .then(|| {
            Some((
                count.immediate32(),
                image.va_to_rva(rows.ip_rel_memory_address()).ok()?,
            ))
        })
        .flatten()
}

/// A dword stack slot an instruction stores to or loads from, as (base, displacement).
fn stack_slot(instruction: &Instruction) -> Option<(Register, u64)> {
    let memory =
        (0..instruction.op_count()).any(|operand| instruction.op_kind(operand) == OpKind::Memory);
    (memory
        && stack_base(instruction.memory_base())
        && instruction.memory_index() == Register::None
        && instruction.memory_size() == MemorySize::UInt32)
        .then(|| {
            (
                instruction.memory_base(),
                instruction.memory_displacement64(),
            )
        })
}

fn stack_base(register: Register) -> bool {
    matches!(register, Register::RSP | Register::RBP)
}

/// The rows a registry's initializer constructs and the one scope all of them document.
fn decode_registry(
    image: &PeImage,
    registry: &Registry,
    initializer: u32,
) -> io::Result<(Option<NumericScope>, Vec<(String, u32)>)> {
    let (start, end) = image.function(initializer)?;
    let instructions = image.instructions(start, end)?;
    let mut rows = Vec::new();
    let mut ordinals = BTreeSet::new();
    let mut scopes: Option<u8> = None;
    for (at, instruction) in instructions.iter().enumerate() {
        if instruction.mnemonic() != Mnemonic::Call {
            continue;
        }
        let before = instructions
            .get(at.saturating_sub(2)..at)
            .filter(|_| at >= 2)
            .ok_or_else(|| invalid("numeric property registration lacks its operands"))?;
        let row_destination = registry
            .table
            .checked_add(
                (rows.len() as u32)
                    .checked_mul(PROPERTY_ROW_WIDTH)
                    .ok_or_else(|| invalid("numeric property registry span overflows"))?,
            )
            .ok_or_else(|| invalid("numeric property registry destination overflows"))?;
        if before[0].mnemonic() != Mnemonic::Lea
            || before[0].op0_register() != Register::RDX
            || !before[0].is_ip_rel_memory_operand()
            || image.va_to_rva(before[0].ip_rel_memory_address())? != row_destination
            || before[1].mnemonic() != Mnemonic::Lea
            || before[1].op0_register() != Register::RCX
            || !stack_base(before[1].memory_base())
            || instruction.op0_kind() != OpKind::NearBranch64
        {
            return Err(invalid(
                "numeric property registration has unsupported operands",
            ));
        }
        let constructor = image.va_to_rva(instruction.near_branch_target())?;
        let (name, ordinal, documented) = decode_row(image, constructor)?;
        if !ordinals.insert(ordinal) {
            return Err(invalid(
                "numeric property registry repeats a handler ordinal",
            ));
        }
        scopes = Some(scopes.map_or(documented, |scopes| scopes & documented));
        rows.push((name, ordinal));
    }
    if rows.len() != registry.count as usize {
        return Err(invalid(
            "numeric property registry count differs from its lookup",
        ));
    }
    let scopes = scopes.unwrap_or(0);
    let scope = match scopes.count_ones() {
        0 => None,
        1 => Some(NumericScope::ALL[scopes.trailing_zeros() as usize]),
        _ => return Err(invalid("numeric property registry scope is ambiguous")),
    };
    if scope.is_some() {
        let names: BTreeSet<_> = rows.iter().map(|(name, _)| name).collect();
        if names.len() != rows.len() {
            return Err(invalid("numeric property registry repeats a name"));
        }
    }
    Ok((scope, rows))
}

/// A row constructor's property name and ordinal, and the scopes whose documented name it
/// loads, as a mask.
fn decode_row(image: &PeImage, constructor: u32) -> io::Result<(String, u32, u8)> {
    let (start, end) = image.function(constructor)?;
    if start != constructor {
        return Err(invalid(
            "numeric property constructor is not a complete function",
        ));
    }
    let instructions = image.instructions(start, end)?;
    let mut receiver = false;
    let mut descriptor = None;
    let mut documented = 0;
    for (at, instruction) in instructions.iter().enumerate() {
        if instruction.op0_kind() == OpKind::Register
            && matches!(
                instruction.op0_register(),
                Register::RBX | Register::EBX | Register::BX | Register::BL | Register::BH
            )
            && !matches!(
                instruction.mnemonic(),
                Mnemonic::Cmp | Mnemonic::Test | Mnemonic::Push
            )
        {
            receiver = instruction.mnemonic() == Mnemonic::Mov
                && instruction.op0_register() == Register::RBX
                && instruction.op1_kind() == OpKind::Register
                && instruction.op1_register() == Register::RDX;
        }
        if instruction.is_ip_rel_memory_operand()
            && let Ok(text) = image.va_to_rva(instruction.ip_rel_memory_address())
        {
            for scope in NumericScope::ALL {
                if is_text(image, text, scope.documented()) {
                    documented |= 1 << scope as u8;
                }
            }
        }
        let Some(next) = instructions.get(at + 1) else {
            continue;
        };
        if descriptor_copy(instruction, next) {
            if !receiver || descriptor.is_some() {
                return Err(invalid(
                    "numeric property descriptor has an ambiguous receiver",
                ));
            }
            descriptor = Some(image.va_to_rva(instruction.ip_rel_memory_address())?);
        }
    }
    let descriptor =
        descriptor.ok_or_else(|| invalid("numeric property constructor has no descriptor copy"))?;
    let name = image.string(image.va_to_rva(image.u64(descriptor)?)?)?;
    let ordinal = u32::try_from(image.u64(add(descriptor, 8)?)?)
        .map_err(|_| invalid("numeric property handler ordinal exceeds its field"))?;
    if name.is_empty() {
        return Err(invalid("numeric property name is empty"));
    }
    Ok((name.to_owned(), ordinal, documented))
}

fn descriptor_copy(load: &Instruction, store: &Instruction) -> bool {
    load.mnemonic() == Mnemonic::Movups
        && load.op0_kind() == OpKind::Register
        && load.is_ip_rel_memory_operand()
        && store.mnemonic() == Mnemonic::Movups
        && receiver_memory(store, Register::RBX)
        && store.op1_kind() == OpKind::Register
        && store.op1_register() == load.op0_register()
}

fn receiver_memory(instruction: &Instruction, receiver: Register) -> bool {
    instruction.op0_kind() == OpKind::Memory
        && instruction.memory_base() == receiver
        && instruction.memory_index() == Register::None
        && instruction.memory_displacement64() == 0
}

/// Sorted vtables of every implementation of the numeric callback's callable type.
fn callback_vtables(image: &PeImage) -> io::Result<Vec<u32>> {
    let mut locators = Vec::new();
    let mut names = HashMap::new();
    let mut primaries: Vec<_> = image
        .classes()
        .iter()
        .filter(|class| class.offset == 0)
        .map(|class| class.locator)
        .collect();
    primaries.dedup();
    for locator in primaries {
        for (descriptor, _) in image.locator_bases(locator)?.into_iter().skip(1) {
            if type_hash(image, &mut names, descriptor)? == CALLBACK_TYPE {
                locators.push(locator);
                break;
            }
        }
    }
    let found: Vec<(u32, u32)> = image
        .classes()
        .iter()
        .filter(|class| locators.binary_search(&class.locator).is_ok())
        .map(|class| (class.locator, class.vtable))
        .collect();
    let mut owners: Vec<_> = found.iter().map(|&(locator, _)| locator).collect();
    owners.sort_unstable();
    if locators.is_empty() || owners != locators {
        return Err(invalid("numeric callback types missing or repeated"));
    }
    let mut vtables: Vec<_> = found.iter().map(|&(_, vtable)| vtable).collect();
    vtables.sort_unstable();
    Ok(vtables)
}

fn decode_callbacks(
    image: &PeImage,
    start: u32,
    end: u32,
    vtables: &[u32],
    properties: &mut Vec<NumericProperty>,
) -> io::Result<()> {
    let instructions = image.instructions(start, end)?;
    let mut pending = None;
    let mut callback_names = BTreeSet::new();
    let mut registered = BTreeSet::new();
    for instruction in instructions {
        if matches!(instruction.mnemonic(), Mnemonic::Movsd | Mnemonic::Movups)
            && instruction.op1_kind() == OpKind::Memory
            && instruction.is_ip_rel_memory_operand()
        {
            let source = image.va_to_rva(instruction.ip_rel_memory_address())?;
            // A name is copied from its start; later pieces load from inside it.
            if source
                .checked_sub(1)
                .is_some_and(|before| image.u8(before).ok() == Some(0))
            {
                if pending.is_some() {
                    return Err(invalid("numeric callback name has no handler identity"));
                }
                pending = Some(image.string(source)?.to_owned());
            }
        }
        if instruction.mnemonic() != Mnemonic::Lea
            || instruction.op0_register() != Register::RAX
            || !instruction.is_ip_rel_memory_operand()
        {
            continue;
        }
        let name = pending
            .take()
            .ok_or_else(|| invalid("numeric callback handler has no name"))?;
        if !callback_names.insert(name.clone()) {
            return Err(invalid("numeric callback repeats a name"));
        }
        let vtable_rva = image.va_to_rva(instruction.ip_rel_memory_address())?;
        if vtables.binary_search(&vtable_rva).is_err() || !registered.insert(vtable_rva) {
            return Err(invalid("numeric callback handler is not one callback type"));
        }
        let value_reader_rva = executable_pointer(image, add(vtable_rva, CALLBACK_VALUE_READER)?)?;
        let input_reader_rva = executable_pointer(image, add(vtable_rva, CALLBACK_INPUT_READER)?)?;
        properties.push(NumericProperty {
            name: leak_string(name),
            identity: NumericIdentity::Callback {
                vtable_rva,
                value_reader_rva,
                input_reader_rva,
            },
        });
    }
    if pending.is_some() {
        return Err(invalid("numeric callback name has no handler identity"));
    }
    if registered.len() != vtables.len() {
        return Err(invalid("numeric callback type has no registered name"));
    }
    Ok(())
}

fn executable_pointer(image: &PeImage, at: u32) -> io::Result<u32> {
    let pointer = image.u64(at)?;
    if !image.is_code_pointer(pointer) {
        return Err(invalid(
            "numeric callback reader is outside executable storage",
        ));
    }
    image.va_to_rva(pointer)
}

fn add(rva: u32, offset: u32) -> io::Result<u32> {
    rva.checked_add(offset)
        .ok_or_else(|| invalid("numeric property RVA arithmetic overflows"))
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instructions(bytes: &[u8]) -> Vec<Instruction> {
        let mut decoder = Decoder::with_ip(64, bytes, 0x1000, DecoderOptions::NONE);
        let mut instructions = Vec::new();
        while decoder.can_decode() {
            instructions.push(decoder.decode());
        }
        instructions
    }

    #[test]
    fn descriptor_copy_requires_the_same_register_and_complete_receiver() {
        let valid = instructions(&[0x0f, 0x10, 0x05, 0, 0, 0, 0, 0x0f, 0x11, 0x03]);
        assert!(descriptor_copy(&valid[0], &valid[1]));
        let embedded = instructions(&[0x0f, 0x10, 0x05, 0, 0, 0, 0, 0x0f, 0x11, 0x43, 0x10]);
        assert!(!descriptor_copy(&embedded[0], &embedded[1]));
        let different = instructions(&[0x0f, 0x10, 0x05, 0, 0, 0, 0, 0x0f, 0x11, 0x0b]);
        assert!(!descriptor_copy(&different[0], &different[1]));
    }

    #[test]
    fn installed_numeric_properties_cover_every_scope_and_callback() {
        let tables = super::super::initialize_for_tests().expect(
            "numeric property parity requires BATTLE_PLAN_GAME_DIR with the supported installation",
        );
        let properties = &tables.numeric.as_ref().unwrap().properties;
        assert_eq!(properties.len(), 94);
        assert_eq!(
            properties
                .iter()
                .map(|property| property.name)
                .collect::<BTreeSet<_>>()
                .len(),
            73
        );
        for scope in NumericScope::ALL {
            assert!(properties.iter().any(|property| matches!(
                property.identity,
                NumericIdentity::Scope { scope: found, .. } if found == scope
            )));
        }
        assert!(
            properties
                .iter()
                .any(|property| matches!(property.identity, NumericIdentity::Callback { .. }))
        );
    }
}
