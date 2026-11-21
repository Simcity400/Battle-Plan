
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fmt::Write,
    io,
};

use iced_x86::{
    Decoder, DecoderOptions, FlowControl, Instruction, InstructionInfoFactory, MemorySize,
    Mnemonic, OpAccess, OpKind, Register,
};
use memchr::{memchr_iter, memmem};
use sha2::{Digest, Sha256};

use super::{leak_string, pe::PeImage, verify_digest};

const MODIFIER_ALTERNATIVES: usize = 4;
const KEYWORD_SHAPE: &[u8] = &[0x8b, 0x10, 0x48, 0x8d, 0x0d];
const MODIFIER_SHAPE: &[u8] = &[0x8b, 0x08, 0xe8];
const CALL: u8 = 0xe8;

pub const KEYWORDS_DIGEST: &str =
    "bdcff1acaf45b0562819380c1c2c0dab053d16e9b18374545e8645f9ef0813fb";
pub const MODIFIERS_DIGEST: &str =
    "2c399037105446b0acbb08d9865541f94942f8894accd6ba4f91a100e0e57345";

#[derive(Debug)]
pub struct KeywordTables {
    pub names: Vec<(u32, &'static str)>,
    pub modifier_names: Vec<&'static str>,
    folded: HashMap<Vec<u8>, (u32, &'static str)>,
    pub(super) fixed_keywords: Result<(), String>,
    pub(super) fixed_modifiers: Result<(), String>,
    pub(super) content: [u8; 32],
}

impl KeywordTables {
    pub fn content_digest(&self) -> [u8; 32] {
        self.content
    }

    pub fn decode(image: &PeImage) -> io::Result<Self> {
        let keywords = locate_keywords(image)?;
        verify_token_initializer(image, keywords.0.token)?;
        verify_function_start(image, keywords.0.registration, "keywords")?;
        let (names, assignments) = std::thread::scope(|scope| {
            let modifiers = scope.spawn(|| read_modifier_table(image, &keywords));
            let names = read_keyword_table(image, &keywords);
            (
                names,
                modifiers
                    .join()
                    .unwrap_or_else(|_| Err(unlocated("modifiers", "its decoder stopped"))),
            )
        });
        let names = names?;
        let modifiers = resolve_modifiers(&names, &reduce_modifier_groups(&assignments?)?)?;

        let (fixed_keywords, fixed_modifiers, content) = fixed_ids(&names, &modifiers);

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
            fixed_keywords,
            fixed_modifiers,
            content,
        })
    }

    pub(super) fn recorded(
        names: Vec<(u32, &'static str)>,
        modifier_names: Vec<&'static str>,
    ) -> Self {
        let rows = |rows: &mut dyn Iterator<Item = (u32, &str)>| {
            rows.map(|(id, name)| (id, name.to_owned())).collect::<Vec<_>>()
        };
        let (fixed_keywords, fixed_modifiers, content) = fixed_ids(
            &rows(&mut names.iter().copied()),
            &rows(&mut modifier_names.iter().enumerate().map(|(key, name)| (key as u32, *name))),
        );
        let folded = names
            .iter()
            .map(|&(id, name)| {
                let lowered = name.to_ascii_lowercase();
                let lowered_name = if lowered == name {
                    name
                } else {
                    super::leak_string(lowered.clone())
                };
                (lowered.into_bytes(), (id, lowered_name))
            })
            .collect();
        Self {
            names,
            modifier_names,
            folded,
            fixed_keywords,
            fixed_modifiers,
            content,
        }
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

    pub fn fixed_modifiers(&self) -> io::Result<&[&'static str]> {
        self.fixed_modifiers
            .as_ref()
            .map(|()| &self.modifier_names[..])
            .map_err(|reason| unpinned("modifier indices", reason))
    }

    pub fn fixed_ids(&self) -> io::Result<()> {
        self.fixed_keywords
            .as_ref()
            .map(|_| ())
            .map_err(|reason| unpinned("token ids", reason))
    }

    pub fn fixed_token(&self, id: u32) -> io::Result<&'static str> {
        self.fixed_keywords
            .as_ref()
            .map_err(|reason| unpinned("token ids", reason))?;
        self.by_id(id).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("fixed token {id} is absent from the keyword table"),
            )
        })
    }

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

fn unpinned(what: &str, reason: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("{what} this release names in code do not hold for this build: {reason}"),
    )
}

fn invalid(table: &str, occurrence: usize, reason: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("{table} occurrence {occurrence}: {reason}"),
    )
}

fn unlocated(table: &str, reason: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("{table}: {reason}"))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Located {
    function: (u32, u32),
    token: u32,
    registration: u32,
}

fn call_target(code: &[u8], at: usize, site: u32) -> Option<u32> {
    let relative = i32::from_le_bytes(code.get(at + 1..at + 5)?.try_into().ok()?);
    u32::try_from(i64::from(site) + 5 + i64::from(relative)).ok()
}

fn executable_code(image: &PeImage) -> io::Result<Vec<(u32, &[u8])>> {
    image
        .executable_ranges()
        .iter()
        .map(|&(start, end)| Ok((start, image.bytes(start, (end - start) as usize)?)))
        .collect()
}

fn only<K>(table: &str, shape: &str, groups: HashMap<K, usize>) -> io::Result<(K, usize)> {
    let count = groups.len();
    let mut groups = groups.into_iter();
    match (groups.next(), count) {
        (Some(group), 1) => Ok(group),
        (None, _) => Err(unlocated(table, format!("no function holds {shape}"))),
        _ => Err(unlocated(
            table,
            format!("{count} functions or callees hold {shape}"),
        )),
    }
}

fn locate_keywords(image: &PeImage) -> io::Result<(Located, usize)> {
    let mut groups = HashMap::new();
    for (start, code) in executable_code(image)? {
        for at in memmem::find_iter(code, KEYWORD_SHAPE) {
            if at < 12
                || code[at - 12] != CALL
                || code[at - 7..at - 4] != [0x4c, 0x8d, 0x05]
                || code.get(at + 9) != Some(&CALL)
            {
                continue;
            }
            let site = start + at as u32;
            let (Some(token), Some(registration), Ok(function)) = (
                call_target(code, at - 12, site - 12),
                call_target(code, at + 9, site + 9),
                image.function(site),
            ) else {
                continue;
            };
            *groups
                .entry(Located {
                    function,
                    token,
                    registration,
                })
                .or_default() += 1;
        }
    }
    only("keywords", "the keyword registration shape", groups)
}

fn locate_modifiers(image: &PeImage, keywords: Located) -> io::Result<(Located, usize)> {
    let mut groups = HashMap::<_, usize>::new();
    for (start, code) in executable_code(image)? {
        for at in memmem::find_iter(code, MODIFIER_SHAPE) {
            let keyed =
                (at >= 2 && code[at - 2..at] == [0x33, 0xd2]) || (at >= 5 && code[at - 5] == 0xba);
            let site = start + at as u32;
            let (true, Some(registration), Ok(function)) = (
                keyed,
                call_target(code, at + 2, site + 2),
                image.function(site),
            ) else {
                continue;
            };
            if function != keywords.function {
                *groups.entry((function, registration)).or_default() += 1;
            }
        }
    }
    let mut candidates = HashMap::with_capacity(1);
    for ((function, registration), sites) in groups {
        let (start, end) = function;
        let code = image.bytes(start, (end - start) as usize)?;
        if memchr_iter(CALL, code)
            .any(|at| call_target(code, at, start + at as u32) == Some(keywords.token))
        {
            candidates.insert(
                Located {
                    function,
                    token: keywords.token,
                    registration,
                },
                sites,
            );
        }
    }
    only("modifiers", "the modifier registration shape", candidates)
}

fn verify_token_initializer(image: &PeImage, rva: u32) -> io::Result<()> {
    let bytes = image.bytes(rva, 16)?;
    let mut decoder =
        Decoder::with_ip(64, bytes, image.base + u64::from(rva), DecoderOptions::NONE);
    let [store, copy, ret] = std::array::from_fn(|_| decoder.decode());
    let stores_id = store.mnemonic() == Mnemonic::Mov
        && store.op0_kind() == OpKind::Memory
        && store.memory_base() == Register::RCX
        && store.memory_index() == Register::None
        && store.memory_displacement64() == 0
        && store.memory_size() == MemorySize::UInt32
        && store.op1_kind() == OpKind::Register
        && store.op1_register() == Register::EDX;
    let returns_destination = copy.mnemonic() == Mnemonic::Mov
        && copy.op0_kind() == OpKind::Register
        && copy.op0_register() == Register::RAX
        && copy.op1_kind() == OpKind::Register
        && copy.op1_register() == Register::RCX;
    if stores_id && returns_destination && ret.mnemonic() == Mnemonic::Ret && ret.op_count() == 0 {
        Ok(())
    } else {
        Err(unlocated(
            "keywords",
            "the token initializer does not return its stored id",
        ))
    }
}

fn verify_function_start(image: &PeImage, rva: u32, table: &str) -> io::Result<()> {
    if image.function(rva).is_ok_and(|(start, _)| start == rva) {
        Ok(())
    } else {
        Err(unlocated(
            table,
            "the registration callee is not a function start",
        ))
    }
}

fn read_keyword_table(
    image: &PeImage,
    (located, sites): &(Located, usize),
) -> io::Result<Vec<(u32, String)>> {
    let (start, end) = located.function;
    let rows = read_keywords(
        image,
        &image.instructions(start, end)?,
        located.token,
        located.registration,
    )?;
    if rows.len() != *sites {
        return Err(unlocated(
            "keywords",
            format!(
                "{} registrations differ from {sites} registration shapes",
                rows.len()
            ),
        ));
    }
    validate_keywords(rows)
}

fn read_modifier_table(
    image: &PeImage,
    (keywords, _): &(Located, usize),
) -> io::Result<Vec<ModifierAssignment>> {
    let (located, sites) = locate_modifiers(image, *keywords)?;
    verify_function_start(image, located.registration, "modifiers")?;
    let (start, end) = located.function;
    let assignments = read_modifiers(
        image,
        &image.instructions(start, end)?,
        located.token,
        located.registration,
        end,
    )?;
    if assignments.len() != sites {
        return Err(unlocated(
            "modifiers",
            format!(
                "{} registrations differ from {sites} registration shapes",
                assignments.len()
            ),
        ));
    }
    Ok(assignments)
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
        if !image.is_read_only(source, 4) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "token id source is not read-only data",
            ));
        }
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

fn read_keywords(
    image: &PeImage,
    instructions: &[Instruction],
    token_initializer: u32,
    registration: u32,
) -> io::Result<Vec<(u32, String)>> {
    let mut source = None;
    let mut initialized = None;
    let mut argument_ready = false;
    let mut name = None;
    let mut rows = Vec::new();
    let mut info = InstructionInfoFactory::new();
    for instruction in instructions {
        if let Some(id) = load_source(image, instruction)? {
            if source.replace(id).is_some() || initialized.is_some() || argument_ready {
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
            let at = image.va_to_rva(instruction.ip_rel_memory_address())?;
            let literal = image.string(at)?;
            if !image.is_read_only(at, literal.len() + 1) {
                return Err(invalid(
                    "keywords",
                    rows.len(),
                    "name is not read-only data",
                ));
            }
            if name.replace(literal.to_owned()).is_some() {
                return Err(invalid("keywords", rows.len(), "overlapping name argument"));
            }
        } else if instruction.mnemonic() == Mnemonic::Call {
            match direct_target(image, instruction)? {
                Some(target) if target == token_initializer => {
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
                Some(target) if target == registration => {
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
        } else {
            let mut live = Vec::new();
            if source.is_some() || argument_ready {
                live.push(Register::RDX);
            }
            let token_in_memory = initialized.is_some() && !argument_ready;
            if token_in_memory {
                live.push(Register::RAX);
            }
            if name.is_some() {
                live.push(Register::R8);
            }
            if interferes(&mut info, instruction, &live, token_in_memory) {
                return Err(invalid(
                    "keywords",
                    rows.len(),
                    "an instruction changes a registration argument",
                ));
            }
        }
    }
    if source.is_some() || initialized.is_some() || name.is_some() || argument_ready {
        return Err(invalid("keywords", rows.len(), "unfinished registration"));
    }
    Ok(rows)
}

fn interferes(
    info: &mut InstructionInfoFactory,
    instruction: &Instruction,
    live: &[Register],
    memory: bool,
) -> bool {
    if live.is_empty() && !memory {
        return false;
    }
    if instruction.flow_control() != FlowControl::Next {
        return true;
    }
    let written = |access: OpAccess| {
        matches!(
            access,
            OpAccess::Write | OpAccess::CondWrite | OpAccess::ReadWrite | OpAccess::ReadCondWrite
        )
    };
    let details = info.info(instruction);
    details.used_registers().iter().any(|used| {
        written(used.access()) && live.contains(&used.register().full_register())
    }) || (memory && details.used_memory().iter().any(|used| written(used.access())))
}

fn modifier_index(instruction: &Instruction) -> Option<u32> {
    if instruction.op0_kind() != OpKind::Register || instruction.op0_register() != Register::EDX {
        return None;
    }
    match (instruction.mnemonic(), instruction.op1_kind()) {
        (Mnemonic::Mov, OpKind::Immediate32) => Some(instruction.immediate32()),
        (Mnemonic::Xor, OpKind::Register) if instruction.op1_register() == Register::EDX => Some(0),
        _ => None,
    }
}

#[derive(Clone, Default)]
struct Frame {
    registers: Vec<(Register, i64)>,
    slots: Vec<(i64, i64)>,
}

impl Frame {
    fn register(&self, register: Register) -> Option<i64> {
        self.registers
            .iter()
            .find(|(known, _)| *known == register)
            .map(|&(_, offset)| offset)
    }

    fn step(&mut self, info: &mut InstructionInfoFactory, instruction: &Instruction) {
        let stack = |instruction: &Instruction| {
            (instruction.memory_base() == Register::RSP
                && instruction.memory_index() == Register::None)
                .then(|| instruction.memory_displacement64() as i64)
        };
        let mut defined = None;
        match instruction.mnemonic() {
            Mnemonic::Lea if instruction.op0_kind() == OpKind::Register => {
                if let Some(offset) = stack(instruction) {
                    defined = Some((instruction.op0_register(), offset));
                }
            }
            Mnemonic::Mov
                if instruction.op0_kind() == OpKind::Register
                    && instruction.op1_kind() == OpKind::Memory
                    && instruction.memory_size() == MemorySize::UInt64 =>
            {
                if let Some(slot) = stack(instruction)
                    && let Some(&(_, offset)) = self.slots.iter().find(|(at, _)| *at == slot)
                {
                    defined = Some((instruction.op0_register(), offset));
                }
            }
            _ => {}
        }
        let stored = (instruction.mnemonic() == Mnemonic::Mov
            && instruction.op0_kind() == OpKind::Memory
            && instruction.op1_kind() == OpKind::Register
            && instruction.memory_size() == MemorySize::UInt64)
            .then(|| stack(instruction).zip(self.register(instruction.op1_register())))
            .flatten();
        let details = info.info(instruction);
        let written = |access: OpAccess| {
            matches!(
                access,
                OpAccess::Write | OpAccess::CondWrite | OpAccess::ReadWrite | OpAccess::ReadCondWrite
            )
        };
        if details.used_registers().iter().any(|used| {
            written(used.access()) && used.register().full_register() == Register::RSP
        }) {
            *self = Self::default();
            return;
        }
        for used in details.used_registers() {
            if written(used.access()) {
                let full = used.register().full_register();
                self.registers.retain(|(known, _)| *known != full);
            }
        }
        for used in details.used_memory() {
            if written(used.access()) && used.base() == Register::RSP {
                let at = used.displacement() as i64;
                let end = at + used.memory_size().size() as i64;
                self.slots.retain(|&(slot, _)| slot + 8 <= at || end <= slot);
            }
        }
        if instruction.flow_control() == FlowControl::Call {
            for volatile in [
                Register::RAX,
                Register::RCX,
                Register::RDX,
                Register::R8,
                Register::R9,
                Register::R10,
                Register::R11,
            ] {
                self.registers.retain(|(known, _)| *known != volatile);
            }
        }
        if let Some((register, offset)) = defined {
            self.registers.push((register.full_register(), offset));
        }
        if let Some((slot, offset)) = stored {
            self.slots.push((slot, offset));
        }
    }
}

fn writes_slot(info: &mut InstructionInfoFactory, instruction: &Instruction, slot: i64) -> bool {
    info.info(instruction).used_memory().iter().any(|used| {
        let written = matches!(
            used.access(),
            OpAccess::Write | OpAccess::CondWrite | OpAccess::ReadWrite | OpAccess::ReadCondWrite
        );
        if !written {
            return false;
        }
        if used.base() != Register::RSP || used.index() != Register::None {
            return true;
        }
        let at = used.displacement() as i64;
        at < slot + 4 && slot < at + used.memory_size().size() as i64
    })
}

fn validate_keywords(mut rows: Vec<(u32, String)>) -> io::Result<Vec<(u32, String)>> {
    if rows.is_empty() {
        return Err(unlocated("keywords", "the initializer registers nothing"));
    }
    rows.sort_unstable_by_key(|(id, _)| *id);
    let mut folded = HashSet::with_capacity(rows.len());
    for (occurrence, (id, name)) in rows.iter().enumerate() {
        if name.is_empty() || !name.bytes().all(|byte| byte.is_ascii_graphic()) {
            return Err(invalid(
                "keywords",
                occurrence,
                "name is empty or not graphic ASCII",
            ));
        }
        if occurrence > 0 && rows[occurrence - 1].0 == *id {
            return Err(invalid("keywords", occurrence, "duplicate token ID"));
        }
        if !folded.insert(name.to_ascii_lowercase()) {
            return Err(invalid("keywords", occurrence, "duplicate folded name"));
        }
    }
    Ok(rows)
}

#[derive(Clone, Copy, Debug)]
struct ModifierAssignment {
    key: u32,
    token: u32,
    join: u32,
}

fn read_modifiers(
    image: &PeImage,
    instructions: &[Instruction],
    token_initializer: u32,
    registration: u32,
    end: u32,
) -> io::Result<Vec<ModifierAssignment>> {
    let mut source = None;
    let mut initialized = None;
    let mut argument_ready = false;
    let mut key = None;
    let mut assignments = Vec::new();
    let mut info = InstructionInfoFactory::new();
    let mut frame = Frame::default();
    let mut token_slot = None;
    for (index, instruction) in instructions.iter().enumerate() {
        let tracked = frame.clone();
        frame.step(&mut info, instruction);
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
        } else if let Some(index) = modifier_index(instruction) {
            if source.is_some() || (initialized.is_some() && key.is_some()) {
                return Err(invalid(
                    "modifiers",
                    assignments.len(),
                    "an instruction changes a registration argument",
                ));
            }
            key = Some(index);
        } else if instruction.mnemonic() == Mnemonic::Call {
            match direct_target(image, instruction)? {
                Some(target) if target == token_initializer => {
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
                    token_slot = Some(tracked.register(Register::RCX).ok_or_else(|| {
                        invalid(
                            "modifiers",
                            assignments.len(),
                            "token storage is not a known stack slot",
                        )
                    })?);
                    key = None;
                }
                Some(target) if target == registration => {
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
                    let join = modifier_join(
                        image,
                        &instructions[index + 1..],
                        registration,
                        end,
                        assignments.len(),
                    )?;
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
        } else {
            let mut live = Vec::new();
            if source.is_some() || (initialized.is_some() && key.is_some()) {
                live.push(Register::RDX);
            }
            let token_in_memory = initialized.is_some() && !argument_ready;
            if token_in_memory {
                live.extend([Register::RAX, Register::RSP]);
            }
            if argument_ready {
                live.push(Register::RCX);
            }
            let writes_token = token_in_memory
                && token_slot.is_none_or(|slot| writes_slot(&mut info, instruction, slot));
            if interferes(&mut info, instruction, &live, false) || writes_token {
                return Err(invalid(
                    "modifiers",
                    assignments.len(),
                    "an instruction changes a registration argument",
                ));
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
    Ok(assignments)
}

fn resolve_modifiers(
    keywords: &[(u32, String)],
    effective: &BTreeMap<u32, u32>,
) -> io::Result<Vec<(u32, String)>> {
    if effective.is_empty() {
        return Err(unlocated("modifiers", "the initializer registers nothing"));
    }
    let mut tokens = HashSet::with_capacity(effective.len());
    effective
        .iter()
        .enumerate()
        .map(|(occurrence, (&key, &token))| {
            if key as usize != occurrence {
                return Err(invalid(
                    "modifiers",
                    occurrence,
                    "noncontiguous fixed indices",
                ));
            }
            if !tokens.insert(token) {
                return Err(invalid("modifiers", occurrence, "duplicate token"));
            }
            let index = keywords
                .binary_search_by_key(&token, |(id, _)| *id)
                .map_err(|_| {
                    invalid(
                        "modifiers",
                        occurrence,
                        "registration names an unknown token",
                    )
                })?;
            Ok((key, keywords[index].1.clone()))
        })
        .collect()
}

fn modifier_join(
    image: &PeImage,
    suffix: &[Instruction],
    registration: u32,
    end: u32,
    occurrence: usize,
) -> io::Result<u32> {
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
            if target <= site || target >= end {
                return Err(invalid(
                    "modifiers",
                    occurrence,
                    "alternative has an invalid continuation",
                ));
            }
            return Ok(target);
        }
        if instruction.mnemonic() == Mnemonic::Call
            && direct_target(image, instruction)? == Some(registration)
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
        effective.insert(first.key, first.token);
    }
    Ok(effective)
}

fn fixed_ids(
    names: &[(u32, String)],
    modifiers: &[(u32, String)],
) -> (Result<(), String>, Result<(), String>, [u8; 32]) {
    let keyword_rows = canonical_rows(names);
    let modifier_rows = canonical_rows(modifiers);
    let fixed_keywords = verify_digest("keywords", keyword_rows.as_bytes(), KEYWORDS_DIGEST)
        .map_err(|error| error.to_string());
    let fixed_modifiers = verify_digest("modifiers", modifier_rows.as_bytes(), MODIFIERS_DIGEST)
        .map_err(|error| error.to_string())
        .and_then(|()| {
            if modifiers.len() == crate::build::FIXED_MODIFIER_GAP as usize {
                Ok(())
            } else {
                Err(format!(
                    "{} fixed modifiers differ from the fixed modifier gap",
                    modifiers.len()
                ))
            }
        });
    let mut content = Sha256::new();
    content.update(keyword_rows.as_bytes());
    content.update(b"\n");
    content.update(modifier_rows.as_bytes());
    (fixed_keywords, fixed_modifiers, content.finalize().into())
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

    const CODE: u32 = 0x1000;
    const DATA: u32 = 0x2000;
    const TOKEN_INITIALIZER: u32 = 0x1180;
    const REGISTRATION: u32 = 0x1190;

    fn synthetic_image(code: &[u8]) -> PeImage {
        let mut bytes = vec![0; 0x600];
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
        word(&mut bytes, 0x86, 2);
        word(&mut bytes, 0x94, 0xf0);
        let optional = 0x98;
        word(&mut bytes, optional, 0x20b);
        bytes[optional + 24..optional + 32]
            .copy_from_slice(&super::super::pe::PREFERRED_IMAGE_BASE.to_le_bytes());
        dword(&mut bytes, optional + 32, 0x1000);
        dword(&mut bytes, optional + 36, 0x200);
        dword(&mut bytes, optional + 56, 0x3000);
        dword(&mut bytes, optional + 60, 0x200);
        dword(&mut bytes, optional + 108, 4);
        dword(&mut bytes, optional + 112 + 3 * 8, DATA + 0x80);
        dword(&mut bytes, optional + 116 + 3 * 8, 24);
        for (index, (name, rva, characteristics)) in [
            (&b".text"[..], CODE, 0x6000_0020),
            (&b".rdata"[..], DATA, 0x4000_0040),
        ]
        .into_iter()
        .enumerate()
        {
            let section = optional + 0xf0 + index * 40;
            bytes[section..section + name.len()].copy_from_slice(name);
            dword(&mut bytes, section + 8, 0x200);
            dword(&mut bytes, section + 12, rva);
            dword(&mut bytes, section + 16, 0x200);
            dword(&mut bytes, section + 20, 0x200 + index as u32 * 0x200);
            dword(&mut bytes, section + 36, characteristics);
        }
        bytes[0x200..0x200 + code.len()].copy_from_slice(code);
        let at = |rva: u32| (rva - CODE) as usize + 0x200;
        bytes[at(TOKEN_INITIALIZER)..at(TOKEN_INITIALIZER) + 6]
            .copy_from_slice(&[0x89, 0x11, 0x48, 0x8b, 0xc1, 0xc3]);
        bytes[at(REGISTRATION)] = 0xc3;
        dword(&mut bytes, 0x400, 17);
        bytes[0x404..0x40e].copy_from_slice(b"Synthetic\0");
        for (index, (start, end)) in [
            (CODE, CODE + code.len() as u32),
            (REGISTRATION, REGISTRATION + 1),
        ]
        .into_iter()
        .enumerate()
        {
            let entry = 0x480 + index * 12;
            dword(&mut bytes, entry, start);
            dword(&mut bytes, entry + 4, end);
            dword(&mut bytes, entry + 8, DATA + 0xa0);
        }
        PeImage::parse(bytes).unwrap()
    }

    fn keyword_code(initialize: bool) -> Vec<u8> {
        fn relative(code: &mut Vec<u8>, opcode: &[u8], target: u32) {
            code.extend_from_slice(opcode);
            let next = CODE + code.len() as u32 + 4;
            code.extend_from_slice(&(target as i32 - next as i32).to_le_bytes());
        }
        let mut code = Vec::new();
        relative(&mut code, &[0x8b, 0x15], DATA);
        if initialize {
            relative(&mut code, &[0xe8], TOKEN_INITIALIZER);
        }
        relative(&mut code, &[0x4c, 0x8d, 0x05], DATA + 4);
        code.extend_from_slice(&[0x8b, 0x10]);
        relative(&mut code, &[0x48, 0x8d, 0x0d], DATA + 0x40);
        relative(&mut code, &[0xe8], REGISTRATION);
        code.push(0xc3);
        code
    }

    fn decoded(image: &PeImage, code: &[u8]) -> io::Result<Vec<(u32, String)>> {
        let instructions = image.instructions(CODE, CODE + code.len() as u32).unwrap();
        read_keywords(image, &instructions, TOKEN_INITIALIZER, REGISTRATION)
    }

    #[test]
    fn keyword_registration_requires_paired_initialization() {
        let code = keyword_code(true);
        let image = synthetic_image(&code);
        assert_eq!(decoded(&image, &code).unwrap(), [(17, "Synthetic".into())]);
        let code = keyword_code(false);
        let image = synthetic_image(&code);
        assert!(decoded(&image, &code).is_err());
    }

    #[test]
    fn keywords_are_located_by_shape_and_verified_callees() {
        let code = keyword_code(true);
        let image = synthetic_image(&code);
        let (located, sites) = locate_keywords(&image).unwrap();
        assert_eq!(
            located,
            Located {
                function: (CODE, CODE + code.len() as u32),
                token: TOKEN_INITIALIZER,
                registration: REGISTRATION,
            }
        );
        assert_eq!(sites, 1);
        verify_token_initializer(&image, located.token).unwrap();
        verify_function_start(&image, located.registration, "keywords").unwrap();
        assert!(verify_token_initializer(&image, REGISTRATION).is_err());
        assert!(locate_keywords(&synthetic_image(&keyword_code(false))).is_err());
    }

    #[test]
    fn duplicate_folded_keywords_are_refused() {
        let rows = vec![(1, "entry".into()), (2, "ENTRY".into())];
        assert!(validate_keywords(rows).is_err());
        assert!(validate_keywords(vec![(1, "entry".into()), (1, "other".into())]).is_err());
        assert!(validate_keywords(vec![(1, "two words".into())]).is_err());
        assert_eq!(
            validate_keywords(vec![(2, "b".into()), (1, "a".into())]).unwrap(),
            [(1, "a".into()), (2, "b".into())]
        );
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
    fn modifier_indices_are_dense_and_distinct() {
        let keywords = vec![(11, "first".into()), (12, "second".into())];
        let named = resolve_modifiers(&keywords, &[(0, 12), (1, 11)].into()).unwrap();
        assert_eq!(named, [(0, "second".into()), (1, "first".into())]);
        assert!(resolve_modifiers(&keywords, &[(0, 11), (2, 12)].into()).is_err());
        assert!(resolve_modifiers(&keywords, &[(0, 11), (1, 11)].into()).is_err());
        assert!(resolve_modifiers(&keywords, &[(0, 13)].into()).is_err());
        assert!(resolve_modifiers(&keywords, &BTreeMap::new()).is_err());
    }

    #[test]
    fn keyword_lookup_folds_ascii_and_stops_at_nul() {
        let tables = KeywordTables {
            names: vec![(17, "SyntheticEntry")],
            modifier_names: Vec::new(),
            folded: [(b"syntheticentry".to_vec(), (17, "syntheticentry"))].into(),
            fixed_keywords: Err("synthetic".into()),
            fixed_modifiers: Err("synthetic".into()),
            content: [0; 32],
        };
        assert_eq!(tables.by_id(17), Some("SyntheticEntry"));
        assert_eq!(
            tables.static_keyword(b"SYNTHETICENTRY\0other"),
            Some((17, "syntheticentry"))
        );
        assert_eq!(tables.static_keyword(b"syntheticentry\xff"), None);
        assert!(tables.fixed_token(17).is_err());
        assert!(tables.fixed_modifiers().is_err());
    }

    #[test]
    #[ignore = "requires BATTLE_PLAN_GAME_DIR"]
    fn installed_keyword_digest() {
        let tables = super::super::initialize_for_tests()
            .expect("set BATTLE_PLAN_GAME_DIR to the supported installation");
        let keywords = tables.keywords.as_ref().unwrap();
        assert!(keywords.fixed_keywords.is_ok());
        let rows: Vec<_> = keywords
            .names
            .iter()
            .map(|&(id, name)| (id, name.to_owned()))
            .collect();
        verify_digest(
            "keywords",
            canonical_rows(&rows).as_bytes(),
            KEYWORDS_DIGEST,
        )
        .unwrap();
    }

    #[test]
    #[ignore = "requires BATTLE_PLAN_GAME_DIR"]
    fn installed_modifier_digest() {
        let tables = super::super::initialize_for_tests()
            .expect("set BATTLE_PLAN_GAME_DIR to the supported installation");
        let keywords = tables.keywords.as_ref().unwrap();
        let modifiers: Vec<_> = keywords
            .fixed_modifiers()
            .unwrap()
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
