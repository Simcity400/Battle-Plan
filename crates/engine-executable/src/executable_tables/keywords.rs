//! Immutable keyword identities and fixed modifier names from the supported executable.

use std::{
    collections::{BTreeMap, HashMap},
    fmt::Write,
    io,
};

use iced_x86::{Instruction, MemorySize, Mnemonic, OpKind, Register};

use super::{leak_string, pe::PeImage, verify_digest};

const KEYWORD_INITIALIZER_START: u32 = 0x0008_3760;
const KEYWORD_INITIALIZER_END: u32 = 0x0011_9491;
const TOKEN_ID_INITIALIZER: u32 = 0x0015_3750;
const KEYWORD_REGISTRATION: u32 = 0x0254_7630;
const MODIFIER_INITIALIZER_START: u32 = 0x005a_66a0;
const MODIFIER_INITIALIZER_END: u32 = 0x0065_3127;
const MODIFIER_REGISTRATION: u32 = 0x0059_e360;
const MODIFIER_ALTERNATIVES: usize = 4;

pub const KEYWORDS_DIGEST: &str =
    "bdcff1acaf45b0562819380c1c2c0dab053d16e9b18374545e8645f9ef0813fb";
pub const MODIFIERS_DIGEST: &str =
    "2c399037105446b0acbb08d9865541f94942f8894accd6ba4f91a100e0e57345";

#[derive(Debug)]
pub struct KeywordTables {
    /// Original spellings in increasing token order.
    pub names: Vec<(u32, &'static str)>,
    /// Effective fixed modifier names in modifier-index order.
    pub modifier_names: Vec<&'static str>,
    folded: HashMap<Vec<u8>, (u32, &'static str)>,
}

impl KeywordTables {
    pub fn decode(image: &PeImage) -> io::Result<Self> {
        for (table, start, end) in [
            (
                "keywords",
                KEYWORD_INITIALIZER_START,
                KEYWORD_INITIALIZER_END,
            ),
            (
                "modifiers",
                MODIFIER_INITIALIZER_START,
                MODIFIER_INITIALIZER_END,
            ),
        ] {
            if image.function(start)? != (start, end) {
                return Err(invalid(table, 0, "initializer has an unsupported extent"));
            }
        }
        let names = read_keywords(
            image,
            &image.instructions(KEYWORD_INITIALIZER_START, KEYWORD_INITIALIZER_END)?,
        )?;
        verify_digest(
            "keywords",
            canonical_rows(&names).as_bytes(),
            KEYWORDS_DIGEST,
        )?;

        let modifiers = read_modifiers(
            image,
            &names,
            &image.instructions(MODIFIER_INITIALIZER_START, MODIFIER_INITIALIZER_END)?,
        )?;
        verify_digest(
            "modifiers",
            canonical_rows(&modifiers).as_bytes(),
            MODIFIERS_DIGEST,
        )?;
        if modifiers.len() != crate::build::FIXED_MODIFIER_GAP as usize
            || modifiers
                .iter()
                .enumerate()
                .any(|(index, (key, _))| *key as usize != index)
        {
            return Err(invalid(
                "modifiers",
                modifiers.len(),
                "noncontiguous fixed indices",
            ));
        }

        let mut folded = HashMap::with_capacity(names.len());
        let mut installed_names = Vec::with_capacity(names.len());
        for (id, name) in names {
            let lowered = name.to_ascii_lowercase();
            let original = leak_string(name);
            let lowered_name = if original == lowered {
                original
            } else {
                leak_string(lowered.clone())
            };
            folded.insert(lowered.into_bytes(), (id, lowered_name));
            installed_names.push((id, original));
        }
        let modifier_names = modifiers
            .into_iter()
            .map(|(_, name)| leak_string(name))
            .collect();
        Ok(Self {
            names: installed_names,
            modifier_names,
            folded,
        })
    }

    pub fn by_id(&self, id: u32) -> Option<&'static str> {
        self.names
            .binary_search_by_key(&id, |&(token, _)| token)
            .ok()
            .map(|index| self.names[index].1)
    }

    pub fn name(&self, id: u32) -> Option<&'static str> {
        self.by_id(id)
    }

    /// Keywords use ASCII case identity and text before the first NUL.
    pub fn static_keyword(&self, word: &[u8]) -> Option<(u32, &'static str)> {
        let end = word
            .iter()
            .position(|&byte| byte == 0)
            .unwrap_or(word.len());
        self.folded.get(&word[..end].to_ascii_lowercase()).copied()
    }

    pub fn id(&self, word: &[u8]) -> Option<u32> {
        self.static_keyword(word).map(|(id, _)| id)
    }
}

fn invalid(table: &str, occurrence: usize, reason: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("{table} occurrence {occurrence}: {reason}"),
    )
}

fn direct_target(image: &PeImage, instruction: &Instruction) -> io::Result<Option<u32>> {
    if !matches!(instruction.op0_kind(), OpKind::NearBranch64) {
        return Ok(None);
    }
    image.va_to_rva(instruction.near_branch_target()).map(Some)
}

fn load_source(image: &PeImage, instruction: &Instruction) -> io::Result<Option<u32>> {
    if instruction.mnemonic() == Mnemonic::Mov
        && instruction.op0_kind() == OpKind::Register
        && instruction.op0_register() == Register::EDX
        && instruction.op1_kind() == OpKind::Memory
        && instruction.is_ip_rel_memory_operand()
        && instruction.memory_size() == MemorySize::UInt32
    {
        let source = image.va_to_rva(instruction.ip_rel_memory_address())?;
        return image.u32(source).map(Some);
    }
    Ok(None)
}

fn is_returned_id(instruction: &Instruction, destination: Register) -> bool {
    instruction.mnemonic() == Mnemonic::Mov
        && instruction.op0_kind() == OpKind::Register
        && instruction.op0_register() == destination
        && instruction.op1_kind() == OpKind::Memory
        && instruction.memory_base() == Register::RAX
        && instruction.memory_index() == Register::None
        && instruction.memory_displacement64() == 0
        && instruction.memory_size() == MemorySize::UInt32
}

fn read_keywords(image: &PeImage, instructions: &[Instruction]) -> io::Result<Vec<(u32, String)>> {
    let mut source = None;
    let mut initialized = None;
    let mut argument_ready = false;
    let mut name = None;
    let mut rows = Vec::new();
    for instruction in instructions {
        if let Some(id) = load_source(image, instruction)? {
            if source.replace(id).is_some() || initialized.is_some() {
                return Err(invalid(
                    "keywords",
                    rows.len(),
                    "overlapping token initialization",
                ));
            }
        } else if is_returned_id(instruction, Register::EDX) {
            if initialized.is_none() || argument_ready {
                return Err(invalid("keywords", rows.len(), "unpaired token argument"));
            }
            argument_ready = true;
        } else if instruction.mnemonic() == Mnemonic::Lea
            && instruction.op0_kind() == OpKind::Register
            && instruction.op0_register() == Register::R8
            && instruction.is_ip_rel_memory_operand()
        {
            let literal = image.string(image.va_to_rva(instruction.ip_rel_memory_address())?)?;
            if name.replace(literal.to_owned()).is_some() {
                return Err(invalid("keywords", rows.len(), "overlapping name argument"));
            }
        } else if instruction.mnemonic() == Mnemonic::Call {
            match direct_target(image, instruction)? {
                Some(TOKEN_ID_INITIALIZER) => {
                    let id = source.take().ok_or_else(|| {
                        invalid("keywords", rows.len(), "token initializer has no source")
                    })?;
                    if initialized.replace(id).is_some() {
                        return Err(invalid(
                            "keywords",
                            rows.len(),
                            "reused token initialization",
                        ));
                    }
                }
                Some(KEYWORD_REGISTRATION) => {
                    let id = initialized.take().ok_or_else(|| {
                        invalid(
                            "keywords",
                            rows.len(),
                            "registration has no initialized token",
                        )
                    })?;
                    let literal = name.take().ok_or_else(|| {
                        invalid("keywords", rows.len(), "registration has no name")
                    })?;
                    if !argument_ready || source.is_some() {
                        return Err(invalid(
                            "keywords",
                            rows.len(),
                            "registration arguments are unpaired",
                        ));
                    }
                    argument_ready = false;
                    rows.push((id, literal));
                }
                _ if source.is_some() || initialized.is_some() || name.is_some() => {
                    return Err(invalid(
                        "keywords",
                        rows.len(),
                        "call interrupts registration arguments",
                    ));
                }
                _ => {}
            }
        }
    }
    if source.is_some() || initialized.is_some() || name.is_some() || argument_ready {
        return Err(invalid("keywords", rows.len(), "unfinished registration"));
    }
    validate_keywords(rows)
}

fn validate_keywords(mut rows: Vec<(u32, String)>) -> io::Result<Vec<(u32, String)>> {
    rows.sort_unstable_by_key(|(id, _)| *id);
    let mut folded = HashMap::with_capacity(rows.len());
    for (occurrence, (id, name)) in rows.iter().enumerate() {
        if name.is_empty() || !name.is_ascii() {
            return Err(invalid(
                "keywords",
                occurrence,
                "name is empty or non-ASCII",
            ));
        }
        if occurrence > 0 && rows[occurrence - 1].0 == *id {
            return Err(invalid("keywords", occurrence, "duplicate token ID"));
        }
        if folded.insert(name.to_ascii_lowercase(), *id).is_some() {
            return Err(invalid("keywords", occurrence, "duplicate folded name"));
        }
    }
    Ok(rows)
}

#[derive(Clone, Copy)]
struct ModifierAssignment {
    key: u32,
    token: u32,
    join: u32,
}

fn read_modifiers(
    image: &PeImage,
    keywords: &[(u32, String)],
    instructions: &[Instruction],
) -> io::Result<Vec<(u32, String)>> {
    let mut source = None;
    let mut initialized = None;
    let mut argument_ready = false;
    let mut key = None;
    let mut assignments = Vec::new();
    for (index, instruction) in instructions.iter().enumerate() {
        if let Some(token) = load_source(image, instruction)? {
            if source.replace(token).is_some() || initialized.is_some() {
                return Err(invalid(
                    "modifiers",
                    assignments.len(),
                    "overlapping token initialization",
                ));
            }
        } else if is_returned_id(instruction, Register::ECX) {
            if initialized.is_none() || argument_ready {
                return Err(invalid(
                    "modifiers",
                    assignments.len(),
                    "unpaired token argument",
                ));
            }
            argument_ready = true;
        } else if instruction.mnemonic() == Mnemonic::Mov
            && instruction.op0_kind() == OpKind::Register
            && instruction.op0_register() == Register::EDX
            && instruction.op1_kind() == OpKind::Immediate32
        {
            key = Some(instruction.immediate32());
        } else if instruction.mnemonic() == Mnemonic::Xor
            && instruction.op0_kind() == OpKind::Register
            && instruction.op1_kind() == OpKind::Register
            && instruction.op0_register() == Register::EDX
            && instruction.op1_register() == Register::EDX
        {
            key = Some(0);
        } else if instruction.mnemonic() == Mnemonic::Call {
            match direct_target(image, instruction)? {
                Some(TOKEN_ID_INITIALIZER) => {
                    let token = source.take().ok_or_else(|| {
                        invalid(
                            "modifiers",
                            assignments.len(),
                            "token initializer has no source",
                        )
                    })?;
                    if initialized.replace(token).is_some() {
                        return Err(invalid(
                            "modifiers",
                            assignments.len(),
                            "reused token initialization",
                        ));
                    }
                    key = None;
                }
                Some(MODIFIER_REGISTRATION) => {
                    let token = initialized.take().ok_or_else(|| {
                        invalid(
                            "modifiers",
                            assignments.len(),
                            "registration has no initialized token",
                        )
                    })?;
                    let key = key.take().ok_or_else(|| {
                        invalid(
                            "modifiers",
                            assignments.len(),
                            "registration has no modifier index",
                        )
                    })?;
                    if !argument_ready || source.is_some() {
                        return Err(invalid(
                            "modifiers",
                            assignments.len(),
                            "registration arguments are unpaired",
                        ));
                    }
                    argument_ready = false;
                    let join = modifier_join(image, &instructions[index + 1..], assignments.len())?;
                    assignments.push(ModifierAssignment { key, token, join });
                }
                _ if source.is_some() || initialized.is_some() => {
                    return Err(invalid(
                        "modifiers",
                        assignments.len(),
                        "call interrupts registration arguments",
                    ));
                }
                _ => key = None,
            }
        }
    }
    if source.is_some() || initialized.is_some() || argument_ready {
        return Err(invalid(
            "modifiers",
            assignments.len(),
            "unfinished registration",
        ));
    }
    let effective = reduce_modifier_groups(&assignments)?;
    effective
        .into_iter()
        .map(|(key, token)| {
            let index = keywords
                .binary_search_by_key(&token, |(id, _)| *id)
                .map_err(|_| {
                    invalid(
                        "modifiers",
                        key as usize,
                        "registration names an unknown token",
                    )
                })?;
            Ok((key, keywords[index].1.clone()))
        })
        .collect()
}

fn modifier_join(image: &PeImage, suffix: &[Instruction], occurrence: usize) -> io::Result<u32> {
    for instruction in suffix {
        if instruction.mnemonic() == Mnemonic::Jmp {
            let target = direct_target(image, instruction)?.ok_or_else(|| {
                invalid(
                    "modifiers",
                    occurrence,
                    "alternative has an indirect continuation",
                )
            })?;
            let site = image.va_to_rva(instruction.ip())?;
            if target <= site || target >= MODIFIER_INITIALIZER_END {
                return Err(invalid(
                    "modifiers",
                    occurrence,
                    "alternative has an invalid continuation",
                ));
            }
            return Ok(target);
        }
        if instruction.mnemonic() == Mnemonic::Call
            && direct_target(image, instruction)? == Some(MODIFIER_REGISTRATION)
        {
            break;
        }
        if instruction.mnemonic() == Mnemonic::Ret
            || matches!(instruction.op0_kind(), OpKind::NearBranch64)
                && instruction.mnemonic() != Mnemonic::Call
        {
            break;
        }
    }
    Err(invalid(
        "modifiers",
        occurrence,
        "alternative has no shared continuation",
    ))
}

fn reduce_modifier_groups(assignments: &[ModifierAssignment]) -> io::Result<BTreeMap<u32, u32>> {
    let mut effective = BTreeMap::new();
    for (occurrence, group) in assignments.chunks(MODIFIER_ALTERNATIVES).enumerate() {
        let first = group[0];
        if group.len() != MODIFIER_ALTERNATIVES
            || group.iter().any(|row| {
                row.join != first.join || row.key != first.key || row.token != first.token
            })
        {
            return Err(invalid(
                "modifiers",
                occurrence,
                "conditional registrations disagree",
            ));
        }
        // Each alternative reaches the same continuation. Later groups replace earlier names.
        effective.insert(first.key, first.token);
    }
    Ok(effective)
}

fn canonical_rows(rows: &[(u32, String)]) -> String {
    let mut canonical = String::new();
    for (id, name) in rows {
        writeln!(canonical, "{id}\t{name}").expect("writing into a string cannot fail");
    }
    canonical
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic_image(code: &[u8]) -> PeImage {
        let mut bytes = vec![0; 0x400];
        let word = |bytes: &mut [u8], offset: usize, value: u16| {
            bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
        };
        let dword = |bytes: &mut [u8], offset: usize, value: u32| {
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        };
        bytes[..2].copy_from_slice(b"MZ");
        dword(&mut bytes, 0x3c, 0x80);
        bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
        word(&mut bytes, 0x84, 0x8664);
        word(&mut bytes, 0x86, 1);
        word(&mut bytes, 0x94, 0xf0);
        let optional = 0x98;
        word(&mut bytes, optional, 0x20b);
        bytes[optional + 24..optional + 32]
            .copy_from_slice(&super::super::pe::PREFERRED_IMAGE_BASE.to_le_bytes());
        dword(&mut bytes, optional + 32, 0x1000);
        dword(&mut bytes, optional + 36, 0x200);
        dword(
            &mut bytes,
            optional + 56,
            (KEYWORD_REGISTRATION & !0xfff) + 0x1000,
        );
        dword(&mut bytes, optional + 60, 0x200);
        dword(&mut bytes, optional + 108, 4);
        dword(&mut bytes, optional + 112 + 3 * 8, 0x1180);
        dword(&mut bytes, optional + 116 + 3 * 8, 12);
        let section = optional + 0xf0;
        bytes[section..section + 5].copy_from_slice(b".text");
        dword(&mut bytes, section + 8, 0x200);
        dword(&mut bytes, section + 12, 0x1000);
        dword(&mut bytes, section + 16, 0x200);
        dword(&mut bytes, section + 20, 0x200);
        dword(&mut bytes, section + 36, 0x6000_0020);
        bytes[0x200..0x200 + code.len()].copy_from_slice(code);
        dword(&mut bytes, 0x300, 17);
        bytes[0x304..0x30e].copy_from_slice(b"Synthetic\0");
        dword(&mut bytes, 0x380, 0x1000);
        dword(&mut bytes, 0x384, 0x1000 + code.len() as u32);
        dword(&mut bytes, 0x388, 0x11a0);
        PeImage::parse(bytes).unwrap()
    }

    fn keyword_code(initialize: bool) -> Vec<u8> {
        fn relative(code: &mut Vec<u8>, opcode: &[u8], target: u32) {
            code.extend_from_slice(opcode);
            let next = 0x1000 + code.len() as u32 + 4;
            code.extend_from_slice(&(target as i32 - next as i32).to_le_bytes());
        }
        let mut code = Vec::new();
        relative(&mut code, &[0x8b, 0x15], 0x1100);
        if initialize {
            relative(&mut code, &[0xe8], TOKEN_ID_INITIALIZER);
        }
        relative(&mut code, &[0x4c, 0x8d, 0x05], 0x1104);
        code.extend_from_slice(&[0x8b, 0x10]);
        relative(&mut code, &[0xe8], KEYWORD_REGISTRATION);
        code.push(0xc3);
        code
    }

    #[test]
    fn keyword_registration_requires_paired_initialization() {
        let code = keyword_code(true);
        let image = synthetic_image(&code);
        let instructions = image
            .instructions(0x1000, 0x1000 + code.len() as u32)
            .unwrap();
        assert_eq!(
            read_keywords(&image, &instructions).unwrap(),
            [(17, "Synthetic".into())]
        );
        let code = keyword_code(false);
        let image = synthetic_image(&code);
        let instructions = image
            .instructions(0x1000, 0x1000 + code.len() as u32)
            .unwrap();
        assert!(read_keywords(&image, &instructions).is_err());
    }

    #[test]
    fn duplicate_folded_keywords_are_refused() {
        let rows = vec![(1, "entry".into()), (2, "ENTRY".into())];
        assert!(validate_keywords(rows).is_err());
        assert!(validate_keywords(vec![(1, "entry".into()), (1, "other".into())]).is_err());
    }

    #[test]
    fn modifier_alternatives_must_agree_before_later_overwrite() {
        let initial = ModifierAssignment {
            key: 7,
            token: 11,
            join: 100,
        };
        let later = ModifierAssignment {
            key: 7,
            token: 12,
            join: 200,
        };
        let mut rows = vec![initial; MODIFIER_ALTERNATIVES];
        rows.extend([later; MODIFIER_ALTERNATIVES]);
        assert_eq!(reduce_modifier_groups(&rows).unwrap().get(&7), Some(&12));
        rows[1].token = 13;
        assert!(reduce_modifier_groups(&rows).is_err());
        rows[1] = initial;
        rows[2].join = 101;
        assert!(reduce_modifier_groups(&rows).is_err());
        assert!(reduce_modifier_groups(&rows[..3]).is_err());
    }

    #[test]
    fn keyword_lookup_folds_ascii_and_stops_at_nul() {
        let tables = KeywordTables {
            names: vec![(17, "SyntheticEntry")],
            modifier_names: Vec::new(),
            folded: [(b"syntheticentry".to_vec(), (17, "syntheticentry"))].into(),
        };
        assert_eq!(tables.by_id(17), Some("SyntheticEntry"));
        assert_eq!(
            tables.static_keyword(b"SYNTHETICENTRY\0other"),
            Some((17, "syntheticentry"))
        );
        assert_eq!(tables.static_keyword(b"syntheticentry\xff"), None);
    }

    #[test]
    #[ignore = "requires BATTLE_PLAN_GAME_DIR"]
    fn installed_keyword_digest() {
        let tables = super::super::initialize_for_tests()
            .expect("set BATTLE_PLAN_GAME_DIR to the supported installation");
        let keywords: Vec<_> = tables
            .keywords
            .names
            .iter()
            .map(|&(id, name)| (id, name.to_owned()))
            .collect();
        verify_digest(
            "keywords",
            canonical_rows(&keywords).as_bytes(),
            KEYWORDS_DIGEST,
        )
        .unwrap();
    }

    #[test]
    #[ignore = "requires BATTLE_PLAN_GAME_DIR"]
    fn installed_modifier_digest() {
        let tables = super::super::initialize_for_tests()
            .expect("set BATTLE_PLAN_GAME_DIR to the supported installation");
        let modifiers: Vec<_> = tables
            .keywords
            .modifier_names
            .iter()
            .enumerate()
            .map(|(index, &name)| (index as u32, name.to_owned()))
            .collect();
        verify_digest(
            "modifiers",
            canonical_rows(&modifiers).as_bytes(),
            MODIFIERS_DIGEST,
        )
        .unwrap();
    }
}
