//! Immutable rule, equipment and AI metadata from the supported executable.

use super::{keywords::KeywordTables, leak_string, pe::PeImage, verify_digest};
use iced_x86::{Instruction, Mnemonic, OpKind, Register};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

const CATEGORY_START: u32 = 0x00f7_31b0;
const CATEGORY_END: u32 = 0x00f7_33b8;
const CATEGORY_ZERO_CASE: u32 = 0x00f7_334c;
const UNNAMED_CATEGORY: u64 = 0x80_0000;
const STAT_START: u32 = 0x013d_2e60;
const STAT_END: u32 = 0x013d_321a;
const STAT_LABELS: u32 = 0x0309_b3e0;
const STAT_POLARITY: u32 = 0x0309_b650;
const STAT_COUNT: usize = 78;
const AI_START: u32 = 0x0149_7260;
const AI_END: u32 = 0x0149_7e9a;
const MISSION_START: u32 = 0x00fa_efe0;
const MISSION_END: u32 = 0x00fa_f0cf;
const EXECUTION_START: u32 = 0x0149_8240;
const EXECUTION_END: u32 = 0x0149_8297;
const ORDER_START: u32 = 0x0065_5a7a;
const ORDER_END: u32 = 0x0065_5a87;
const SELECTOR_START: u32 = 0x0053_0bd4;
const SELECTOR_END: u32 = 0x0053_0cff;
const RULE_START: u32 = 0x0062_dbc0;
const RULE_END: u32 = 0x0062_e8bf;
const RULE_CONSTRUCT: u32 = 0x0062_bfe0;
const STRING_COPY: u32 = 0x0011_db00;
const STRING_ASSIGN: u32 = 0x0011_d460;
const STRING_ALLOCATE: u32 = 0x021b_4b50;
const STRING_RELEASE: u32 = 0x021b_4b60;
const RULE_STRIDE: u64 = 0x38;
const RULE_LOCAL_BASE: u64 = 0x2e0;
const RULE_TOKEN: u64 = 0x28;
const RULE_INDEX: u64 = 0x2c;
const RULE_FULFILLED: u64 = 0x30;
const RULE_DEFAULT: u64 = 0x32;
const OUTPUT: u64 = 0x1000;
const STACK: u64 = 0x10_0000;
const FRAME: u64 = STACK + 0x100;
const HEAP: u64 = 0x30_0000;
const CATEGORY_DIGEST: &str = "0877af49699d2b02647bfa20c438b5af6c2a47cc42893c96e60057fb401bb082";
const CATEGORY_NAMES_DIGEST: &str =
    "eb08b560f635730feefc5db7504ea9a866155d79e83636d3fef519a157b19163";
const STAT_METADATA_DIGEST: &str =
    "7263a410fefbf89683dc2b99faeebb04939a4f130d27f01dcacb1d0a71b61c96";
const STAT_LABELS_DIGEST: &str = "5bdee4598e99cd3c3aa03b5552f865d466dcaa11df3f40c13008ffe7fdf3a083";
const AI_STRATEGY_DIGEST: &str = "7e0063a19cc0f48c059f71fbe43617a6538f5fa73cf8a0524e52998cf0318a01";
const MISSION_DIGEST: &str = "cee379b59c3d429330492d7b5c48cd76ea95ca3a35a6c2979bcadaff8dca438c";
const EXECUTION_DIGEST: &str = "f60168baa0838ba3cbed853370dbef361df76fcebffca2508ea1e6e40977bba1";
const ORDER_DIGEST: &str = "aa1969d161001a30d706c858ac4db4aabeef6e1a4c0adf63167f1a7c84621f61";
const SELECTOR_DIGEST: &str = "cff5d0a220c5cb27009fd781434cbdc5db09733ac80477d2834eff0a436c5c29";
const RULE_DIGEST: &str = "1b72c6a717f7942fa0de6a3f12bec1986fa0c83c0246f634beed95fede545ac2";

#[derive(Debug)]
pub struct AuxiliaryTables {
    pub rules: Vec<(&'static str, &'static str, bool, bool)>,
    pub equipment_categories: Vec<(u32, u64, &'static str)>,
    pub category_names: Vec<(&'static str, u64)>,
    pub stat_metadata: Vec<(&'static str, &'static str, bool)>,
    pub ai_strategy_types: Vec<(&'static str, u32)>,
    pub missions: Vec<(&'static str, u32)>,
    pub execution_types: Vec<(&'static str, u32)>,
    pub order_types: Vec<(&'static str, u32)>,
    pub selectors: Vec<(&'static str, u32)>,
}

impl AuxiliaryTables {
    pub(crate) fn decode(image: &PeImage, keywords: &KeywordTables) -> io::Result<Self> {
        let categories = Dispatch::new(image, CATEGORY_START, CATEGORY_END)?;
        let mut equipment_categories = Vec::new();
        for &(token, name) in &keywords.names {
            let result = categories.select(Register::ECX, token, false)?.0;
            if result != 0 && result != UNNAMED_CATEGORY {
                equipment_categories.push((token, result, name));
            }
        }
        equipment_categories.sort_by_key(|row| row.0);
        if categories.select(Register::ECX, 0, false)?.0 != UNNAMED_CATEGORY {
            return Err(invalid("equipment categories: unnamed sentinel differs"));
        }
        let zero = image.instructions(CATEGORY_ZERO_CASE, CATEGORY_ZERO_CASE + 6)?;
        let comparison = zero
            .first()
            .ok_or_else(|| invalid("equipment categories: zero case is missing"))?;
        if comparison.mnemonic() != Mnemonic::Cmp || comparison.op0_register() != Register::ECX {
            return Err(invalid("equipment categories: zero case pattern differs"));
        }
        let zero_token = immediate(comparison, 1)? as u32;
        let zero_name = keywords
            .name(zero_token)
            .ok_or_else(|| invalid("equipment categories: zero keyword is missing"))?;
        if categories.select(Register::ECX, zero_token, false)?.0 != 0 {
            return Err(invalid("equipment categories: explicit zero differs"));
        }
        let mut category_names = equipment_categories
            .iter()
            .map(|&(_, mask, name)| (name, mask))
            .collect::<Vec<_>>();
        category_names.sort_by_key(|row| row.1);
        category_names.insert(0, (zero_name, 0));

        let stats = Dispatch::new(image, STAT_START, STAT_END)?;
        let mut indexed = BTreeMap::new();
        for &(token, name) in &keywords.names {
            let index = stats.select(Register::ECX, token, false)?.0;
            if index != STAT_COUNT as u64
                && (index >= STAT_COUNT as u64 || indexed.insert(index, name).is_some())
            {
                return Err(invalid("equipment stats: duplicate or invalid index"));
            }
        }
        if indexed.len() != STAT_COUNT {
            return Err(invalid(format!(
                "equipment stats: {} indices, expected {STAT_COUNT}",
                indexed.len()
            )));
        }
        let mut stat_metadata = Vec::new();
        for (index, name) in indexed {
            let label =
                image.string(image.va_to_rva(image.u64(STAT_LABELS + index as u32 * 8)?)?)?;
            let polarity = image.u8(STAT_POLARITY + index as u32)?;
            if polarity > 1 {
                return Err(invalid("equipment stats: invalid polarity"));
            }
            stat_metadata.push((name, leak_string(label), polarity != 0));
        }
        let ai_strategy_types = enum_projection(image, keywords, AI_START, AI_END)?;
        let missions = enum_projection(image, keywords, MISSION_START, MISSION_END)?;
        let execution_types = enum_projection(image, keywords, EXECUTION_START, EXECUTION_END)?;
        let order_types = order_projection(image, keywords)?;
        let selectors = selector_projection(image, keywords)?;
        let rules = rule_projection(image, keywords)?;
        let tables = Self {
            rules,
            equipment_categories,
            category_names,
            stat_metadata,
            ai_strategy_types,
            missions,
            execution_types,
            order_types,
            selectors,
        };
        tables.verify()?;
        Ok(tables)
    }

    fn verify(&self) -> io::Result<()> {
        let text = self
            .equipment_categories
            .iter()
            .map(|(id, mask, name)| format!("{id}\t{mask}\t{name}\n"))
            .collect::<String>();
        verify_digest("equipment categories", text.as_bytes(), CATEGORY_DIGEST)?;
        let text = self
            .category_names
            .iter()
            .map(|(name, mask)| format!("{name}\t{mask}\n"))
            .collect::<String>();
        verify_digest(
            "equipment category names",
            text.as_bytes(),
            CATEGORY_NAMES_DIGEST,
        )?;
        let text = self
            .stat_metadata
            .iter()
            .map(|(name, label, polarity)| format!("{name}\t{label}\t{}\n", boolean(*polarity)))
            .collect::<String>();
        verify_digest(
            "equipment stat metadata",
            text.as_bytes(),
            STAT_METADATA_DIGEST,
        )?;
        let text = self
            .stat_metadata
            .iter()
            .map(|(_, label, polarity)| format!("{label}\t{}\n", boolean(*polarity)))
            .collect::<String>();
        verify_digest("equipment stat labels", text.as_bytes(), STAT_LABELS_DIGEST)?;
        verify_enum(
            "AI strategy types",
            &self.ai_strategy_types,
            AI_STRATEGY_DIGEST,
        )?;
        verify_enum("AI missions", &self.missions, MISSION_DIGEST)?;
        verify_enum(
            "AI execution types",
            &self.execution_types,
            EXECUTION_DIGEST,
        )?;
        verify_enum("AI order types", &self.order_types, ORDER_DIGEST)?;
        verify_enum("AI selectors", &self.selectors, SELECTOR_DIGEST)?;
        let text = self
            .rules
            .iter()
            .map(|(name, label, default, fulfilled)| {
                format!(
                    "{name}\t{label}\t{}\t{}\n",
                    boolean(*default),
                    boolean(*fulfilled)
                )
            })
            .collect::<String>();
        verify_digest("power balance rules", text.as_bytes(), RULE_DIGEST)
    }
}

fn boolean(value: bool) -> &'static str {
    if value { "True" } else { "False" }
}

fn verify_enum(name: &str, rows: &[(&str, u32)], digest: &str) -> io::Result<()> {
    let text = rows
        .iter()
        .map(|(name, id)| format!("{name}\t{id}\n"))
        .collect::<String>();
    verify_digest(name, text.as_bytes(), digest)
}

fn enum_projection(
    image: &PeImage,
    keywords: &KeywordTables,
    start: u32,
    end: u32,
) -> io::Result<Vec<(&'static str, u32)>> {
    let dispatch = Dispatch::new(image, start, end)?;
    let mut rows = Vec::new();
    for &(token, name) in &keywords.names {
        let (_, writes) = dispatch.select(Register::EDX, token, true)?;
        match writes.get(&(OUTPUT + 4)) {
            Some(1) => rows.push((
                name,
                *writes
                    .get(&OUTPUT)
                    .ok_or_else(|| invalid("AI enum: value is missing"))? as u32,
            )),
            Some(0) => {}
            _ => return Err(invalid("AI enum: validity is missing or invalid")),
        }
    }
    rows.sort_by_key(|row| row.0);
    Ok(rows)
}

fn order_projection(
    image: &PeImage,
    keywords: &KeywordTables,
) -> io::Result<Vec<(&'static str, u32)>> {
    let code = image.instructions(ORDER_START, ORDER_END)?;
    let mut base = 0_u32;
    let mut rows = Vec::new();
    for instruction in code {
        if instruction.op0_register() == Register::ECX {
            let token = match instruction.mnemonic() {
                Mnemonic::Sub => {
                    base = base
                        .checked_add(immediate(&instruction, 1)? as u32)
                        .ok_or_else(|| invalid("AI orders: token overflow"))?;
                    base
                }
                Mnemonic::Cmp => base
                    .checked_add(immediate(&instruction, 1)? as u32)
                    .ok_or_else(|| invalid("AI orders: token overflow"))?,
                _ => continue,
            };
            let name = keywords
                .name(token)
                .ok_or_else(|| invalid("AI orders: keyword is missing"))?;
            rows.push((name, token));
        }
    }
    rows.sort_by_key(|row| row.0);
    Ok(rows)
}

fn selector_projection(
    image: &PeImage,
    keywords: &KeywordTables,
) -> io::Result<Vec<(&'static str, u32)>> {
    let code = image.instructions(SELECTOR_START, SELECTOR_END)?;
    let mut pending = None;
    let mut rows = Vec::new();
    for (index, instruction) in code.iter().enumerate() {
        if instruction.mnemonic() == Mnemonic::Cmp && instruction.op0_register() == Register::EAX {
            if pending.is_some() {
                return Err(invalid("AI selectors: unmatched comparison"));
            }
            if code
                .get(index + 1)
                .is_none_or(|next| next.mnemonic() != Mnemonic::Jne)
            {
                return Err(invalid("AI selectors: comparison branch differs"));
            }
            pending = Some(immediate(instruction, 1)? as u32);
        } else if instruction.op0_register() == Register::R8D
            && matches!(instruction.mnemonic(), Mnemonic::Or | Mnemonic::Bts)
        {
            let token = pending
                .take()
                .ok_or_else(|| invalid("AI selectors: result has no comparison"))?;
            let operand = immediate(instruction, 1)? as u32;
            let mask = if instruction.mnemonic() == Mnemonic::Bts {
                1_u32
                    .checked_shl(operand)
                    .ok_or_else(|| invalid("AI selectors: bit exceeds result width"))?
            } else {
                operand
            };
            let name = keywords
                .name(token)
                .ok_or_else(|| invalid("AI selectors: keyword is missing"))?;
            rows.push((name, mask));
        }
    }
    if pending.is_some() {
        return Err(invalid("AI selectors: result is missing"));
    }
    rows.sort_by_key(|row| row.0);
    Ok(rows)
}

/// Decodes only the finite arithmetic and branch representation used by selectors.
struct Dispatch<'a> {
    image: &'a PeImage,
    start: u64,
    code: BTreeMap<u64, Instruction>,
}

impl<'a> Dispatch<'a> {
    fn new(image: &'a PeImage, start: u32, end: u32) -> io::Result<Self> {
        let code = image
            .instructions(start, end)?
            .into_iter()
            .map(|i| (i.ip(), i))
            .collect();
        Ok(Self {
            image,
            start: image.base + u64::from(start),
            code,
        })
    }

    fn select(
        &self,
        input: Register,
        token: u32,
        output: bool,
    ) -> io::Result<(u64, BTreeMap<u64, u64>)> {
        let mut regs = BTreeMap::new();
        set_register(&mut regs, Register::RSP, STACK)?;
        set_register(&mut regs, input, u64::from(token))?;
        if output {
            set_register(&mut regs, Register::RCX, OUTPUT)?;
        }
        let mut writes = BTreeMap::new();
        let mut pc = self.start;
        let mut flags = (false, false, false);
        let mut visited = BTreeSet::new();
        loop {
            if !visited.insert(pc) {
                return Err(invalid("selector: cyclic branch"));
            }
            let i = self
                .code
                .get(&pc)
                .ok_or_else(|| invalid("selector: branch leaves decoded instructions"))?;
            let mut next = i.next_ip();
            let read = |operand| operand_value(self.image, &regs, &writes, i, operand);
            match i.mnemonic() {
                Mnemonic::Mov | Mnemonic::Movzx | Mnemonic::Movsx | Mnemonic::Movsxd => {
                    let mut value = read(1)?;
                    if matches!(i.mnemonic(), Mnemonic::Movsx | Mnemonic::Movsxd) {
                        value = signed(value, operand_width(i, 1)?) as u64;
                    }
                    if i.op0_kind() == OpKind::Register {
                        set_register(&mut regs, i.op0_register(), value)?;
                    } else if i.op0_kind() == OpKind::Memory {
                        let address = memory_address(&regs, i)?;
                        if address >= self.image.base {
                            return Err(invalid("selector: result writes image storage"));
                        }
                        writes.insert(address, value & mask(operand_width(i, 0)?));
                    } else {
                        return Err(invalid("selector: unsupported move destination"));
                    }
                }
                Mnemonic::Lea => {
                    let value = memory_address(&regs, i)?;
                    set_register(&mut regs, i.op0_register(), value)?;
                }
                Mnemonic::Xor if i.op0_register() == i.op1_register() => {
                    set_register(&mut regs, i.op0_register(), 0)?;
                    flags = (true, false, false);
                }
                Mnemonic::Sub | Mnemonic::Add | Mnemonic::Xor | Mnemonic::And | Mnemonic::Or => {
                    let a = read(0)?;
                    let b = read(1)?;
                    let width = operand_width(i, 0)?;
                    let value = match i.mnemonic() {
                        Mnemonic::Sub => a.wrapping_sub(b),
                        Mnemonic::Add => a.wrapping_add(b),
                        Mnemonic::Xor => a ^ b,
                        Mnemonic::And => a & b,
                        _ => a | b,
                    } & mask(width);
                    flags = if i.mnemonic() == Mnemonic::Sub {
                        compare(a, b, width)
                    } else {
                        compare(value, 0, width)
                    };
                    set_register(&mut regs, i.op0_register(), value)?;
                }
                Mnemonic::Cmp => flags = compare(read(0)?, read(1)?, operand_width(i, 0)?),
                Mnemonic::Test => flags = compare(read(0)? & read(1)?, 0, operand_width(i, 0)?),
                Mnemonic::Cmove | Mnemonic::Cmovne => {
                    if (i.mnemonic() == Mnemonic::Cmove) == flags.0 {
                        let value = read(1)?;
                        set_register(&mut regs, i.op0_register(), value)?;
                    }
                }
                Mnemonic::Jmp
                | Mnemonic::Je
                | Mnemonic::Jne
                | Mnemonic::Jg
                | Mnemonic::Jge
                | Mnemonic::Jl
                | Mnemonic::Jle
                | Mnemonic::Ja
                | Mnemonic::Jae
                | Mnemonic::Jb
                | Mnemonic::Jbe => {
                    let (equal, below, less) = flags;
                    let taken = match i.mnemonic() {
                        Mnemonic::Jmp => true,
                        Mnemonic::Je => equal,
                        Mnemonic::Jne => !equal,
                        Mnemonic::Jg => !equal && !less,
                        Mnemonic::Jge => !less,
                        Mnemonic::Jl => less,
                        Mnemonic::Jle => equal || less,
                        Mnemonic::Ja => !equal && !below,
                        Mnemonic::Jae => !below,
                        Mnemonic::Jb => below,
                        _ => equal || below,
                    };
                    if taken {
                        next = read(0)?;
                        self.image.va_to_rva(next)?;
                    }
                }
                Mnemonic::Ret => return Ok((register_value(&regs, Register::RAX)?, writes)),
                Mnemonic::Nop => {}
                _ => {
                    return Err(invalid(format!(
                        "selector: 1 unsupported instruction {:?}",
                        i.mnemonic()
                    )));
                }
            }
            pc = next;
        }
    }
}

fn operand_value(
    image: &PeImage,
    regs: &BTreeMap<Register, u64>,
    writes: &BTreeMap<u64, u64>,
    i: &Instruction,
    operand: u32,
) -> io::Result<u64> {
    match i.op_kind(operand) {
        OpKind::Register => register_value(regs, i.op_register(operand)),
        OpKind::Memory => {
            let address = memory_address(regs, i)?;
            if let Some(value) = writes.get(&address) {
                return Ok(*value & mask(operand_width(i, operand)?));
            }
            let bytes = image.bytes(image.va_to_rva(address)?, operand_width(i, operand)?)?;
            let mut value = [0; 8];
            value[..bytes.len()].copy_from_slice(bytes);
            Ok(u64::from_le_bytes(value))
        }
        OpKind::NearBranch16 | OpKind::NearBranch32 | OpKind::NearBranch64 => {
            Ok(i.near_branch_target())
        }
        _ => immediate(i, operand),
    }
}

fn immediate(i: &Instruction, operand: u32) -> io::Result<u64> {
    if matches!(
        i.op_kind(operand),
        OpKind::Immediate8
            | OpKind::Immediate16
            | OpKind::Immediate32
            | OpKind::Immediate64
            | OpKind::Immediate8to16
            | OpKind::Immediate8to32
            | OpKind::Immediate8to64
            | OpKind::Immediate32to64
    ) {
        Ok(i.immediate(operand))
    } else {
        Err(invalid("metadata: immediate operand is missing"))
    }
}

fn operand_width(i: &Instruction, operand: u32) -> io::Result<usize> {
    let width = match i.op_kind(operand) {
        OpKind::Register => i.op_register(operand).size(),
        OpKind::Memory => i.memory_size().size(),
        _ => 8,
    };
    if width == 0 || width > 8 {
        return Err(invalid("selector: unsupported operand width"));
    }
    Ok(width)
}

fn mask(width: usize) -> u64 {
    if width == 8 {
        u64::MAX
    } else {
        (1_u64 << (width * 8)) - 1
    }
}
fn signed(value: u64, width: usize) -> i64 {
    let shift = 64 - width * 8;
    ((value << shift) as i64) >> shift
}
fn compare(a: u64, b: u64, width: usize) -> (bool, bool, bool) {
    let a = a & mask(width);
    let b = b & mask(width);
    (a == b, a < b, signed(a, width) < signed(b, width))
}

fn register_value(regs: &BTreeMap<Register, u64>, register: Register) -> io::Result<u64> {
    let value = *regs
        .get(&register.full_register())
        .ok_or_else(|| invalid("metadata: register value is unknown"))?;
    let value = if matches!(
        register,
        Register::AH | Register::BH | Register::CH | Register::DH
    ) {
        value >> 8
    } else {
        value
    };
    Ok(value & mask(register.size()))
}

fn set_register(
    regs: &mut BTreeMap<Register, u64>,
    register: Register,
    value: u64,
) -> io::Result<()> {
    let width = register.size();
    if width == 0 || width > 8 {
        return Err(invalid("metadata: unsupported register width"));
    }
    let full = register.full_register();
    let value = value & mask(width);
    let merged = if width >= 4 {
        value
    } else {
        let previous = regs.get(&full).copied().unwrap_or(0);
        let shift = if matches!(
            register,
            Register::AH | Register::BH | Register::CH | Register::DH
        ) {
            8
        } else {
            0
        };
        (previous & !(mask(width) << shift)) | (value << shift)
    };
    regs.insert(full, merged);
    Ok(())
}

fn memory_address(regs: &BTreeMap<Register, u64>, i: &Instruction) -> io::Result<u64> {
    if i.is_ip_rel_memory_operand() {
        return Ok(i.ip_rel_memory_address());
    }
    let base = if i.memory_base() == Register::None {
        0
    } else {
        register_value(regs, i.memory_base())?
    };
    let index = if i.memory_index() == Register::None {
        0
    } else {
        register_value(regs, i.memory_index())?
    };
    Ok(base
        .wrapping_add(index.wrapping_mul(u64::from(i.memory_index_scale())))
        .wrapping_add(i.memory_displacement64()))
}

struct RuleConstruction<'a> {
    image: &'a PeImage,
    regs: BTreeMap<Register, u64>,
    vectors: BTreeMap<Register, Vec<u8>>,
    bytes: BTreeMap<u64, u8>,
    strings: BTreeMap<u64, String>,
    rows: BTreeMap<u32, (u32, String, bool, bool)>,
    next_heap: u64,
}

impl<'a> RuleConstruction<'a> {
    fn new(image: &'a PeImage) -> Self {
        Self {
            image,
            regs: BTreeMap::from([(Register::RSP, STACK), (Register::RBP, FRAME)]),
            vectors: BTreeMap::new(),
            bytes: BTreeMap::new(),
            strings: BTreeMap::new(),
            rows: BTreeMap::new(),
            next_heap: HEAP,
        }
    }

    fn bytes_at(&self, address: u64, length: usize) -> io::Result<Vec<u8>> {
        if address >= self.image.base {
            return Ok(self
                .image
                .bytes(self.image.va_to_rva(address)?, length)?
                .to_vec());
        }
        (0..length)
            .map(|offset| {
                let address = address
                    .checked_add(offset as u64)
                    .ok_or_else(|| invalid("rules: string address overflow"))?;
                self.bytes
                    .get(&address)
                    .copied()
                    .ok_or_else(|| invalid("rules: construction byte is unknown"))
            })
            .collect()
    }

    fn number_at(&self, address: u64, length: usize) -> io::Result<u64> {
        let bytes = self.bytes_at(address, length)?;
        let mut value = [0; 8];
        value[..length].copy_from_slice(&bytes);
        Ok(u64::from_le_bytes(value))
    }

    fn store(&mut self, address: u64, bytes: &[u8]) -> io::Result<()> {
        if address >= self.image.base {
            return Err(invalid("rules: construction writes image storage"));
        }
        for (offset, &byte) in bytes.iter().enumerate() {
            let address = address
                .checked_add(offset as u64)
                .ok_or_else(|| invalid("rules: construction address overflow"))?;
            self.bytes.insert(address, byte);
        }
        Ok(())
    }

    fn string(&self, object: u64) -> io::Result<String> {
        if let Some(value) = self.strings.get(&object) {
            return Ok(value.clone());
        }
        let length = usize::try_from(self.number_at(object + 16, 8)?)
            .map_err(|_| invalid("rules: string length overflow"))?;
        let capacity = self.number_at(object + 24, 8)?;
        if length as u64 > capacity {
            return Err(invalid("rules: string length exceeds capacity"));
        }
        let payload = if capacity <= 15 {
            object
        } else {
            self.number_at(object, 8)?
        };
        let bytes = self.bytes_at(
            payload,
            length
                .checked_add(1)
                .ok_or_else(|| invalid("rules: string extent overflow"))?,
        )?;
        if bytes.last() != Some(&0) || bytes[..length].contains(&0) {
            return Err(invalid("rules: label length or terminator differs"));
        }
        String::from_utf8(bytes[..length].to_vec())
            .map_err(|_| invalid("rules: label is not UTF-8"))
    }

    fn source_bytes(&self, i: &Instruction) -> io::Result<Vec<u8>> {
        let width = if i.op0_kind() == OpKind::Memory {
            i.memory_size().size()
        } else {
            i.op0_register().size()
        };
        if width == 0 || width > 16 {
            return Err(invalid("rules: unsupported construction width"));
        }
        let source_width = if i.op1_kind() == OpKind::Memory {
            i.memory_size().size()
        } else {
            width
        };
        let mut bytes = match i.op1_kind() {
            OpKind::Memory => self.bytes_at(memory_address(&self.regs, i)?, source_width)?,
            OpKind::Register if i.op1_register().size() > 8 => self
                .vectors
                .get(&i.op1_register())
                .cloned()
                .ok_or_else(|| invalid("rules: vector is unknown"))?,
            OpKind::Register => register_value(&self.regs, i.op1_register())?
                .to_le_bytes()
                .to_vec(),
            _ => immediate(i, 1)?.to_le_bytes().to_vec(),
        };
        bytes.resize(width, 0);
        bytes.truncate(width);
        Ok(bytes)
    }

    fn clobber_call(&mut self) {
        for register in [
            Register::RAX,
            Register::RCX,
            Register::RDX,
            Register::R8,
            Register::R9,
            Register::R10,
            Register::R11,
        ] {
            self.regs.remove(&register);
        }
        for register in [
            Register::XMM0,
            Register::XMM1,
            Register::XMM2,
            Register::XMM3,
            Register::XMM4,
            Register::XMM5,
        ] {
            self.vectors.remove(&register);
        }
    }

    fn call(&mut self, target: u32) -> io::Result<()> {
        let mut allocated = None;
        match target {
            STRING_ALLOCATE => {
                let length = register_value(&self.regs, Register::ECX)?;
                let address = self.next_heap;
                self.next_heap = address
                    .checked_add(length)
                    .and_then(|end| end.checked_add(16))
                    .ok_or_else(|| invalid("rules: allocation extent overflow"))?;
                if self.next_heap >= self.image.base {
                    return Err(invalid("rules: allocation overlaps image"));
                }
                allocated = Some(address);
            }
            STRING_RELEASE => {}
            STRING_ASSIGN => {
                let object = register_value(&self.regs, Register::RCX)?;
                let address = register_value(&self.regs, Register::RDX)?;
                let length = usize::try_from(register_value(&self.regs, Register::R8D)?)
                    .map_err(|_| invalid("rules: label length overflow"))?;
                let bytes = self.bytes_at(address, length)?;
                if bytes.contains(&0) {
                    return Err(invalid("rules: assigned label contains a terminator"));
                }
                let text = String::from_utf8(bytes)
                    .map_err(|_| invalid("rules: assigned label is not UTF-8"))?;
                self.strings.insert(object, text);
            }
            STRING_COPY => {
                let source = register_value(&self.regs, Register::RDX)?;
                let destination = register_value(&self.regs, Register::RCX)?;
                self.strings.insert(destination, self.string(source)?);
            }
            RULE_CONSTRUCT => {
                let index = register_value(&self.regs, Register::R8D)? as u32;
                let token = register_value(&self.regs, Register::EDX)? as u32;
                let object = register_value(&self.regs, Register::RCX)?;
                let expected = FRAME + RULE_LOCAL_BASE + u64::from(index) * RULE_STRIDE;
                if object != expected {
                    return Err(invalid("rules: index and destination disagree"));
                }
                let label = self.string(register_value(&self.regs, Register::R9)?)?;
                let default = self.number_at(STACK + 0x30, 1)?;
                let fulfilled = self.number_at(STACK + 0x20, 1)?;
                if default > 1 || fulfilled > 1 {
                    return Err(invalid("rules: flag is not boolean"));
                }
                if self
                    .rows
                    .insert(index, (token, label, default != 0, fulfilled != 0))
                    .is_some()
                {
                    return Err(invalid("rules: duplicate constructor index"));
                }
            }
            _ => return Err(invalid("rules: 1 unsupported construction call")),
        }
        self.clobber_call();
        if let Some(address) = allocated {
            set_register(&mut self.regs, Register::RAX, address)?;
        }
        Ok(())
    }

    fn instruction(&mut self, i: &Instruction) -> io::Result<()> {
        match i.mnemonic() {
            Mnemonic::Mov | Mnemonic::Movzx | Mnemonic::Movups | Mnemonic::Movsd => {
                // Cleanup operands outside a constructed label do not supply metadata.
                let value = self.source_bytes(i);
                match i.op0_kind() {
                    OpKind::Register if i.op0_register().size() > 8 => {
                        if let Ok(bytes) = value {
                            self.vectors.insert(i.op0_register(), bytes);
                        } else {
                            self.vectors.remove(&i.op0_register());
                        }
                    }
                    OpKind::Register => {
                        if let Ok(bytes) = value {
                            let mut scalar = [0; 8];
                            scalar[..bytes.len()].copy_from_slice(&bytes);
                            set_register(
                                &mut self.regs,
                                i.op0_register(),
                                u64::from_le_bytes(scalar),
                            )?;
                        } else {
                            self.regs.remove(&i.op0_register().full_register());
                        }
                    }
                    OpKind::Memory => {
                        if let (Ok(address), Ok(bytes)) = (memory_address(&self.regs, i), value) {
                            self.store(address, &bytes)?;
                        }
                    }
                    _ => return Err(invalid("rules: unsupported move destination")),
                }
            }
            Mnemonic::Lea if i.op0_register() != Register::RBP => {
                if let Ok(address) = memory_address(&self.regs, i) {
                    set_register(&mut self.regs, i.op0_register(), address)?;
                } else {
                    self.regs.remove(&i.op0_register().full_register());
                }
            }
            Mnemonic::Xor if i.op0_register() == i.op1_register() => {
                set_register(&mut self.regs, i.op0_register(), 0)?
            }
            Mnemonic::Xorps if i.op0_register() == i.op1_register() => {
                self.vectors.insert(i.op0_register(), vec![0; 16]);
            }
            Mnemonic::Call
                if matches!(i.op0_kind(), OpKind::NearBranch64 | OpKind::NearBranch32) =>
            {
                self.call(self.image.va_to_rva(i.near_branch_target())?)?
            }
            Mnemonic::Call => return Err(invalid("rules: indirect construction call")),
            Mnemonic::Lea
            | Mnemonic::Push
            | Mnemonic::Sub
            | Mnemonic::Add
            | Mnemonic::Inc
            | Mnemonic::Cmp
            | Mnemonic::Jne
            | Mnemonic::Jbe
            | Mnemonic::Jb
            | Mnemonic::Ja
            | Mnemonic::Nop => {}
            _ => {
                return Err(invalid(format!(
                    "rules: 1 unsupported construction instruction {:?}",
                    i.mnemonic()
                )));
            }
        }
        Ok(())
    }
}

fn rule_projection(
    image: &PeImage,
    keywords: &KeywordTables,
) -> io::Result<Vec<(&'static str, &'static str, bool, bool)>> {
    let mut construction = RuleConstruction::new(image);
    for instruction in image.instructions(RULE_START, RULE_END)? {
        construction.instruction(&instruction)?;
    }
    let direct_count = *construction
        .rows
        .keys()
        .next()
        .ok_or_else(|| invalid("rules: no constructed rows"))?;
    for index in 0..direct_count {
        let address = FRAME + RULE_LOCAL_BASE + u64::from(index) * RULE_STRIDE;
        let token = construction.number_at(address + RULE_TOKEN, 4)? as u32;
        if construction.number_at(address + RULE_INDEX, 4)? != u64::from(index) {
            return Err(invalid("rules: direct row index differs"));
        }
        let label = construction.string(address + 8)?;
        let default = construction.number_at(address + RULE_DEFAULT, 1)?;
        let fulfilled = construction.number_at(address + RULE_FULFILLED, 1)?;
        if default > 1 || fulfilled > 1 {
            return Err(invalid("rules: direct flag is not boolean"));
        }
        construction
            .rows
            .insert(index, (token, label, default != 0, fulfilled != 0));
    }
    let mut rows = Vec::new();
    for (expected, (index, (token, label, default, fulfilled))) in
        construction.rows.into_iter().enumerate()
    {
        if expected != index as usize {
            return Err(invalid("rules: missing row index"));
        }
        let name = keywords
            .name(token)
            .ok_or_else(|| invalid("rules: keyword is missing"))?;
        rows.push((name, leak_string(label), default, fulfilled));
    }
    Ok(rows)
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(code: &[u8]) -> PeImage {
        let mut bytes = vec![0; 0x600];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
        let p16 = |bytes: &mut [u8], at: usize, value: u16| {
            bytes[at..at + 2].copy_from_slice(&value.to_le_bytes())
        };
        let p32 = |bytes: &mut [u8], at: usize, value: u32| {
            bytes[at..at + 4].copy_from_slice(&value.to_le_bytes())
        };
        p32(&mut bytes, 0x3c, 0x80);
        p16(&mut bytes, 0x84, 0x8664);
        p16(&mut bytes, 0x86, 1);
        p16(&mut bytes, 0x94, 0xf0);
        p16(&mut bytes, 0x98, 0x20b);
        bytes[0xb0..0xb8].copy_from_slice(&super::super::pe::PREFERRED_IMAGE_BASE.to_le_bytes());
        p32(&mut bytes, 0xb8, 0x1000);
        p32(&mut bytes, 0xbc, 0x200);
        p32(&mut bytes, 0xd0, 0x2000);
        p32(&mut bytes, 0xd4, 0x200);
        p32(&mut bytes, 0x104, 5);
        p32(&mut bytes, 0x120, 0x1300);
        p32(&mut bytes, 0x124, 12);
        p32(&mut bytes, 0x190, 0x400);
        p32(&mut bytes, 0x194, 0x1000);
        p32(&mut bytes, 0x198, 0x400);
        p32(&mut bytes, 0x19c, 0x200);
        p32(&mut bytes, 0x1ac, 0x2000_0000);
        p32(&mut bytes, 0x500, 0x1000);
        p32(&mut bytes, 0x504, 0x1000 + code.len() as u32);
        p32(&mut bytes, 0x508, 0x1320);
        bytes[0x200..0x200 + code.len()].copy_from_slice(code);
        PeImage::parse(bytes).unwrap()
    }

    #[test]
    fn enum_validity_accepts_immediate_and_low_byte_register_sources() {
        let immediate = [
            0xc7, 0x01, 7, 0, 0, 0, 0xc6, 0x41, 4, 1, 0x48, 0x89, 0xc8, 0xc3,
        ];
        let register = [
            0xc7, 0x01, 7, 0, 0, 0, 0xba, 1, 0, 0, 0, 0x88, 0x51, 4, 0x48, 0x89, 0xc8, 0xc3,
        ];
        for code in [&immediate[..], &register[..]] {
            let image = image(code);
            let (_, writes) = Dispatch::new(&image, 0x1000, 0x1000 + code.len() as u32)
                .unwrap()
                .select(Register::EDX, 42, true)
                .unwrap();
            assert_eq!(writes.get(&OUTPUT), Some(&7));
            assert_eq!(writes.get(&(OUTPUT + 4)), Some(&1));
        }
    }

    #[test]
    fn selector_refuses_cycles_and_unknown_operations() {
        for code in [&[0xeb, 0xfe][..], &[0xd1, 0xe0, 0xc3][..]] {
            let image = image(code);
            assert!(
                Dispatch::new(&image, 0x1000, 0x1000 + code.len() as u32)
                    .unwrap()
                    .select(Register::ECX, 42, false)
                    .is_err()
            );
        }
    }

    #[test]
    fn rule_label_requires_its_declared_whole_length() {
        let image = image(&[0xc3]);
        let mut construction = RuleConstruction::new(&image);
        let object = STACK + 0x80;
        construction.store(object, b"SYNTHETIC\0").unwrap();
        construction
            .store(object + 16, &9_u64.to_le_bytes())
            .unwrap();
        construction
            .store(object + 24, &15_u64.to_le_bytes())
            .unwrap();
        assert_eq!(construction.string(object).unwrap(), "SYNTHETIC");
        construction
            .store(object + 16, &8_u64.to_le_bytes())
            .unwrap();
        assert!(construction.string(object).is_err());
    }

    fn tables() -> std::sync::Arc<super::super::ExecutableTables> {
        super::super::initialize_for_tests().expect("runtime auxiliary-table digest checks require BATTLE_PLAN_GAME_DIR with the supported installation")
    }

    #[test]
    fn equipment_category_digest() {
        let tables = tables();
        tables.auxiliary.verify().unwrap();
        assert_eq!(tables.auxiliary.equipment_categories.len(), 38);
    }
    #[test]
    fn equipment_category_name_digest() {
        let tables = tables();
        tables.auxiliary.verify().unwrap();
        assert_eq!(tables.auxiliary.category_names.len(), 39);
    }
    #[test]
    fn equipment_stat_metadata_digest() {
        let tables = tables();
        tables.auxiliary.verify().unwrap();
        assert_eq!(tables.auxiliary.stat_metadata.len(), 78);
    }
    #[test]
    fn equipment_stat_label_digest() {
        tables().auxiliary.verify().unwrap();
    }
    #[test]
    fn ai_strategy_type_digest() {
        let tables = tables();
        tables.auxiliary.verify().unwrap();
        assert_eq!(tables.auxiliary.ai_strategy_types.len(), 112);
    }
    #[test]
    fn ai_mission_digest() {
        let tables = tables();
        tables.auxiliary.verify().unwrap();
        assert_eq!(tables.auxiliary.missions.len(), 9);
    }
    #[test]
    fn ai_execution_type_digest() {
        let tables = tables();
        tables.auxiliary.verify().unwrap();
        assert_eq!(tables.auxiliary.execution_types.len(), 4);
    }
    #[test]
    fn ai_order_type_digest() {
        let tables = tables();
        tables.auxiliary.verify().unwrap();
        assert_eq!(tables.auxiliary.order_types.len(), 2);
    }
    #[test]
    fn ai_selector_digest() {
        let tables = tables();
        tables.auxiliary.verify().unwrap();
        assert_eq!(tables.auxiliary.selectors.len(), 9);
    }
    #[test]
    fn power_balance_rule_digest() {
        let tables = tables();
        tables.auxiliary.verify().unwrap();
        assert_eq!(tables.auxiliary.rules.len(), 28);
    }
}
