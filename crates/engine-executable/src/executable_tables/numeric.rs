use super::{keywords::KeywordTables, leak_string, pe::PeImage, verify_digest};
use iced_x86::{Instruction, Mnemonic, OpKind, Register};
use std::{collections::BTreeSet, io};

const PROPERTY_IDENTITIES_DIGEST: &str =
    "8b6907fb2efa4cec3a26e3ebaaaa9702a78980192d2cde544c310556fbfd524c";
const ORDINARY_PROPERTY_NAMES_DIGEST: &str =
    "b64e8347b571c6652372fb1dc02e71dccf271278f98534c5674946956f1573a3";
const CALLBACK_INITIALIZER_RVA: u32 = 0x0007_8650;
const PROPERTY_ROW_WIDTH: u32 = 16;
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

struct Registry {
    scope: NumericScope,
    lookup_rva: u32,
    initializer_rva: u32,
    handler_rva: u32,
}

const REGISTRIES: [Registry; 4] = [
    Registry {
        scope: NumericScope::Country,
        lookup_rva: 0x01a9_0be0,
        initializer_rva: 0x0007_fe80,
        handler_rva: 0x01a8_e6b0,
    },
    Registry {
        scope: NumericScope::State,
        lookup_rva: 0x01a9_88d0,
        initializer_rva: 0x0008_05f0,
        handler_rva: 0x01a9_8630,
    },
    Registry {
        scope: NumericScope::UnitLeader,
        lookup_rva: 0x01a9_b030,
        initializer_rva: 0x0008_0680,
        handler_rva: 0x01a9_a610,
    },
    Registry {
        scope: NumericScope::Character,
        lookup_rva: 0x01a8_6600,
        initializer_rva: 0x0007_fd40,
        handler_rva: 0x01a8_5c60,
    },
];

impl NumericTables {
    pub fn decode(image: &PeImage, _keywords: &KeywordTables) -> io::Result<Self> {
        let mut properties = Vec::new();
        for registry in &REGISTRIES {
            decode_registry(image, registry, &mut properties)?;
        }
        decode_callbacks(image, &mut properties)?;
        let tables = Self { properties };
        verify_digest(
            "numeric property identities",
            &tables.canonical_identities(),
            PROPERTY_IDENTITIES_DIGEST,
        )?;
        verify_digest(
            "numeric ordinary property names",
            &tables.ordinary_names(),
            ORDINARY_PROPERTY_NAMES_DIGEST,
        )?;
        Ok(tables)
    }

    /// Exact spelling and registry order are significant to numeric admission.
    pub fn exact(&self, name: &str) -> Option<&NumericProperty> {
        self.properties
            .iter()
            .find(|property| property.name == name)
    }

    fn canonical_identities(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        for property in &self.properties {
            bytes.push(match property.identity {
                NumericIdentity::Scope { .. } => 0,
                NumericIdentity::Callback { .. } => 1,
            });
            bytes.extend_from_slice(&(property.name.len() as u32).to_le_bytes());
            bytes.extend_from_slice(property.name.as_bytes());
            match property.identity {
                NumericIdentity::Scope {
                    scope,
                    ordinal,
                    handler_rva,
                } => {
                    bytes.push(scope as u8);
                    bytes.extend_from_slice(&ordinal.to_le_bytes());
                    bytes.extend_from_slice(&handler_rva.to_le_bytes());
                }
                NumericIdentity::Callback {
                    vtable_rva,
                    value_reader_rva,
                    input_reader_rva,
                } => {
                    bytes.extend_from_slice(&vtable_rva.to_le_bytes());
                    bytes.extend_from_slice(&value_reader_rva.to_le_bytes());
                    bytes.extend_from_slice(&input_reader_rva.to_le_bytes());
                }
            }
        }
        bytes
    }

    fn ordinary_names(&self) -> Vec<u8> {
        let balance: BTreeSet<_> = self
            .properties
            .iter()
            .filter_map(|property| {
                matches!(
                    property.identity,
                    NumericIdentity::Scope {
                        scope: NumericScope::Country,
                        ordinal: 46..=58,
                        ..
                    }
                )
                .then_some(property.name)
            })
            .collect();
        let names: BTreeSet<_> = self
            .properties
            .iter()
            .map(|property| property.name)
            .filter(|name| !balance.contains(name))
            .collect();
        let mut bytes = Vec::new();
        for name in names {
            bytes.extend_from_slice(name.as_bytes());
            bytes.push(b'\n');
        }
        bytes
    }
}

fn decode_registry(
    image: &PeImage,
    registry: &Registry,
    properties: &mut Vec<NumericProperty>,
) -> io::Result<()> {
    let lookup = image.instructions(registry.lookup_rva, add(registry.lookup_rva, 14)?)?;
    if lookup.len() != 3
        || lookup[0].mnemonic() != Mnemonic::Mov
        || !receiver_memory(&lookup[0], Register::RCX)
        || lookup[0].op1_kind() != OpKind::Immediate32
        || lookup[1].mnemonic() != Mnemonic::Lea
        || lookup[1].op0_register() != Register::RAX
        || !lookup[1].is_ip_rel_memory_operand()
        || lookup[2].mnemonic() != Mnemonic::Ret
    {
        return Err(invalid("numeric property lookup has an unsupported form"));
    }
    let count = lookup[0].immediate32();
    let destination = image.va_to_rva(lookup[1].ip_rel_memory_address())?;
    let (start, end) = image.function(registry.initializer_rva)?;
    if start != registry.initializer_rva {
        return Err(invalid(
            "numeric property initializer is not a complete function",
        ));
    }
    let instructions = image.instructions(start, end)?;
    let mut found = 0_u32;
    let mut ordinals = BTreeSet::new();
    for (at, instruction) in instructions.iter().enumerate() {
        if instruction.mnemonic() != Mnemonic::Call {
            continue;
        }
        let before = instructions
            .get(at.saturating_sub(2)..at)
            .filter(|_| at >= 2)
            .ok_or_else(|| invalid("numeric property registration lacks its operands"))?;
        let row_destination = destination
            .checked_add(
                found
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
            || !matches!(before[1].memory_base(), Register::RBP | Register::RSP)
            || instruction.op0_kind() != OpKind::NearBranch64
        {
            return Err(invalid(
                "numeric property registration has unsupported operands",
            ));
        }
        let constructor = image.va_to_rva(instruction.near_branch_target())?;
        let (name, ordinal) = decode_row(image, constructor)?;
        if !ordinals.insert(ordinal) {
            return Err(invalid(
                "numeric property registry repeats a handler ordinal",
            ));
        }
        properties.push(NumericProperty {
            name: leak_string(name),
            identity: NumericIdentity::Scope {
                scope: registry.scope,
                ordinal,
                handler_rva: registry.handler_rva,
            },
        });
        found = found
            .checked_add(1)
            .ok_or_else(|| invalid("numeric property registry count overflows"))?;
    }
    if found != count {
        return Err(invalid(
            "numeric property registry count differs from its lookup",
        ));
    }
    Ok(())
}

fn decode_row(image: &PeImage, constructor: u32) -> io::Result<(String, u32)> {
    let (start, end) = image.function(constructor)?;
    if start != constructor {
        return Err(invalid(
            "numeric property constructor is not a complete function",
        ));
    }
    let instructions = image.instructions(start, end)?;
    let mut receiver = false;
    let mut descriptor = None;
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
    Ok((name.to_owned(), ordinal))
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

fn decode_callbacks(image: &PeImage, properties: &mut Vec<NumericProperty>) -> io::Result<()> {
    let (start, end) = image.function(CALLBACK_INITIALIZER_RVA)?;
    if start != CALLBACK_INITIALIZER_RVA {
        return Err(invalid(
            "numeric callback initializer is not a complete function",
        ));
    }
    let instructions = image.instructions(start, end)?;
    let mut pending = None;
    let mut callback_names = BTreeSet::new();
    for instruction in instructions {
        if matches!(instruction.mnemonic(), Mnemonic::Movsd | Mnemonic::Movups)
            && instruction.op1_kind() == OpKind::Memory
            && instruction.is_ip_rel_memory_operand()
        {
            let source = image.va_to_rva(instruction.ip_rel_memory_address())?;
            if image.bytes(source, 3)? == b"Get" {
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
    Ok(())
}

fn executable_pointer(image: &PeImage, at: u32) -> io::Result<u32> {
    let rva = image.va_to_rva(image.u64(at)?)?;
    if !image
        .executable_ranges()
        .iter()
        .any(|&(start, end)| (start..end).contains(&rva))
    {
        return Err(invalid(
            "numeric callback reader is outside executable storage",
        ));
    }
    Ok(rva)
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
    use iced_x86::{Decoder, DecoderOptions};

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
    fn installed_numeric_property_identities_match_digest() {
        let tables = super::super::initialize_for_tests().expect(
            "numeric property parity requires BATTLE_PLAN_GAME_DIR with the supported installation",
        );
        assert_eq!(tables.numeric.properties.len(), 94);
        assert_eq!(
            tables
                .numeric
                .properties
                .iter()
                .map(|property| property.name)
                .collect::<BTreeSet<_>>()
                .len(),
            73
        );
        verify_digest(
            "numeric property identities",
            &tables.numeric.canonical_identities(),
            PROPERTY_IDENTITIES_DIGEST,
        )
        .unwrap();
    }

    #[test]
    fn installed_numeric_property_names_match_digest() {
        let tables = super::super::initialize_for_tests().expect(
            "numeric property parity requires BATTLE_PLAN_GAME_DIR with the supported installation",
        );
        assert_eq!(
            tables
                .numeric
                .ordinary_names()
                .iter()
                .filter(|&&byte| byte == b'\n')
                .count(),
            60
        );
        verify_digest(
            "numeric ordinary property names",
            &tables.numeric.ordinary_names(),
            ORDINARY_PROPERTY_NAMES_DIGEST,
        )
        .unwrap();
    }
}
