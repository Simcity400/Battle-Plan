use super::{pe::PeImage, registries::class_name};
use iced_x86::{Decoder, DecoderOptions, Mnemonic, OpKind, Register};
use std::io;

mod owners;
pub use owners::{Object, Source};

const ENVELOPE_WRITER: u32 = 0x022d_9400;
const ENVELOPE_READER: u32 = 0x022d_9160;
const COUNTRY_PERMISSION: u32 = 0x011c_13e0;
const PLAN_PERMISSION: u32 = 0x013f_ed40;
const PLAN_ASSIGNMENT_PERMISSION: u32 = 0x018d_d440;
const MOVEMENT_PERMISSION: u32 = 0x013f_e970;
const MASS_MOVEMENT_PERMISSION: u32 = 0x013f_e910;
const CANCEL_MOVEMENT_PERMISSION: u32 = 0x013f_e790;
const NAVAL_MOVEMENT_PERMISSION: u32 = 0x013e_d3b0;
const AIR_MISSION_PERMISSION: u32 = 0x019d_3640;
const PURE_VIRTUAL_HANDLER: u32 = 0x025c_2320;
const TYPE_SLOT: u32 = 0x58;
const PERMISSION_SLOT: u32 = 0x98;

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
    objects: Vec<(u32, u32)>,
}

impl CommandTables {
    pub(super) fn decode(image: &PeImage) -> io::Result<Self> {
        let writer = (image.base + u64::from(ENVELOPE_WRITER)).to_le_bytes();
        let mut kinds = Vec::new();
        for (section, bytes) in image.data_sections() {
            for offset in (16..bytes.len().saturating_sub(8)).step_by(8) {
                if bytes[offset..offset + 8] != writer {
                    continue;
                }
                let vtable = section + offset as u32 - 16;
                if image.u64(vtable + 32)? != image.base + u64::from(ENVELOPE_READER) {
                    continue;
                }
                let function = image.va_to_rva(image.u64(vtable + TYPE_SLOT)?)?;
                // The abstract envelope every command derives from is never issued.
                if function == PURE_VIRTUAL_HANDLER {
                    continue;
                }
                let name = class_name(image, vtable)?;
                let mut decoder = Decoder::with_ip(
                    64,
                    image.bytes(function, 16)?,
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
                    return Err(invalid(format!("command type reader unsupported: {name}")));
                }
                let token = if constant {
                    Token::Constant(load.immediate32())
                } else {
                    Token::Word(image.va_to_rva(load.ip_rel_memory_address())?)
                };
                let store = if constant { load } else { decoder.decode() };
                let result = decoder.decode();
                let end = decoder.decode();
                if store.mnemonic() != Mnemonic::Mov
                    || store.memory_base() != Register::RDX
                    || store.memory_displacement64() != 0
                    || (!constant && store.op1_register() != Register::EAX)
                    || result.mnemonic() != Mnemonic::Mov
                    || result.op0_register() != Register::RAX
                    || result.op1_register() != Register::RDX
                    || end.mnemonic() != Mnemonic::Ret
                {
                    return Err(invalid(format!("command type reader unsupported: {name}")));
                }
                let permission = image.va_to_rva(image.u64(vtable + PERMISSION_SLOT)?)?;
                let target = match permission {
                    COUNTRY_PERMISSION => Target::Country(0x28),
                    PLAN_PERMISSION => Target::Group(0x28),
                    PLAN_ASSIGNMENT_PERMISSION => Target::Group(0x40),
                    MOVEMENT_PERMISSION => Target::Unit(0x38),
                    MASS_MOVEMENT_PERMISSION => Target::Units(0x40),
                    CANCEL_MOVEMENT_PERMISSION | NAVAL_MOVEMENT_PERMISSION => Target::Units(0x28),
                    AIR_MISSION_PERMISSION => Target::Wings(0x28),
                    _ => owners::target(permission),
                };
                kinds.push(Kind {
                    vtable,
                    name,
                    token,
                    target,
                });
            }
        }
        kinds.sort_unstable_by_key(|kind| kind.vtable);
        if kinds.is_empty()
            || kinds
                .windows(2)
                .any(|pair| pair[0].vtable == pair[1].vtable)
        {
            return Err(invalid("command type catalog missing or duplicated"));
        }
        Ok(Self {
            kinds,
            objects: owners::objects(image)?,
        })
    }

    pub fn object(&self, vtable: u32, expected: Object) -> bool {
        self.objects
            .binary_search_by_key(&vtable, |&(identity, _)| identity)
            .is_ok_and(|index| self.objects[index].1 & expected.mask() != 0)
    }
}

/// FNV-1a of a class name, which identifies supported classes without spelling them.
pub fn class_hash(name: &str) -> u64 {
    name.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3)
    })
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[cfg(test)]
mod tests {
    #[test]
    fn installed_command_catalog_retains_distinct_vtables_and_owned_targets() {
        let tables = super::super::initialize_for_tests().unwrap();
        let kinds = &tables.commands.kinds;
        assert!(!kinds.is_empty());
        assert!(kinds.windows(2).all(|pair| pair[0].vtable < pair[1].vtable));
        assert!(
            kinds
                .iter()
                .any(|kind| matches!(kind.target, super::Target::Unknown))
        );
        assert!(
            kinds
                .iter()
                .any(|kind| matches!(kind.target, super::Target::Country(_)))
        );
        assert!(
            kinds
                .iter()
                .any(|kind| matches!(kind.target, super::Target::Group(_)))
        );
        assert!(
            kinds
                .iter()
                .any(|kind| matches!(kind.target, super::Target::Unit(_)))
        );
        assert!(
            kinds
                .iter()
                .any(|kind| matches!(kind.target, super::Target::Units(_)))
        );
        assert!(
            kinds
                .iter()
                .any(|kind| matches!(kind.target, super::Target::Wings(_)))
        );
    }
}
