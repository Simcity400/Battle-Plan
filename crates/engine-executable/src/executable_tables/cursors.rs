use super::pe::{Anchor, Hashed, PeImage};
use iced_x86::{Decoder, DecoderOptions, FlowControl, Instruction, Mnemonic, OpKind, Register};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::io;

const LOADING_CURSOR_LOG: Anchor = Anchor {
    what: "the cursor loading log message",
    text: Hashed {
        length: 17,
        fnv: 0xaf30_b6e4_180f_3b7b,
    },
};

const STACK: u64 = 0x10_0000;
const FRAME: u64 = STACK + 0x100;
const HEAP: u64 = 0x30_0000;
const STRING_OBJECT: usize = 32;
const MAX_ALLOCATION: u64 = 0x1_0000;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CursorTable {
    pub paths: Vec<String>,
}

impl CursorTable {
    pub fn decode(image: &PeImage) -> io::Result<Self> {
        Self::decode_with_anchor(image, LOADING_CURSOR_LOG)
    }

    fn decode_with_anchor(image: &PeImage, anchor: Anchor) -> io::Result<Self> {
        let strings = image.find_hashed_strings(&[anchor.text]);
        let references: Vec<u32> = image
            .references_to_each(&strings[0])
            .into_iter()
            .flatten()
            .collect();
        let anchor_rva = sole(references, &format!("reference to {}", anchor.what))?;
        let (start, end) = image.function(anchor_rva)?;
        let instructions = image.instructions(start, end)?;
        let anchor_va = image.base + u64::from(anchor_rva);
        let anchor_index = instructions
            .iter()
            .position(|instruction| instruction.ip() == anchor_va)
            .ok_or_else(|| invalid("cursor log reference does not start an instruction"))?;
        let after = &instructions[anchor_index + 1..];

        let plan = locate_builder(image, after)?;
        let builder = &after[plan.start..=plan.last_push];
        let flow = validate_branches(image, builder, plan.push_callee)?;
        let calls = classify_calls(image, builder, plan.vec_slot, plan.push_callee, &flow.guarded)?;

        let mut construction = CursorConstruction::new(
            image,
            slot_address(plan.vec_slot)?,
            plan.push_callee,
            calls.assign_callee,
            calls.alloc_callee,
            calls.release_callees,
        );
        for (index, instruction) in builder.iter().enumerate() {
            for &register in flow.merges.get(&index).into_iter().flatten() {
                construction.regs.remove(&register);
                construction.vectors.remove(&register);
            }
            construction.instruction(instruction)?;
        }
        if construction.paths.is_empty() {
            return Err(invalid("cursor registry pushed no paths"));
        }
        Self::from_paths(construction.paths)
    }

    pub fn from_paths(paths: impl IntoIterator<Item = String>) -> io::Result<Self> {
        let mut validated = Vec::new();
        for path in paths {
            validate_path(&path)?;
            validated.push(path);
        }
        Ok(Self { paths: validated })
    }

    pub fn path(&self, name: &str) -> Result<&str, String> {
        let mut matched: Option<&str> = None;
        for path in &self.paths {
            if path_stem(path) == Some(name) {
                match matched {
                    None => matched = Some(path.as_str()),
                    Some(existing) if existing == path => {}
                    Some(_) => {
                        return Err(format!(
                            "{name}: maps to multiple paths in the recorded cursor registry"
                        ));
                    }
                }
            }
        }
        matched.ok_or_else(|| format!("{name}: not in the recorded cursor registry"))
    }

    pub(super) fn content_digest(&self) -> [u8; 32] {
        let mut digest = Sha256::new();
        digest.update((self.paths.len() as u64).to_le_bytes());
        for path in &self.paths {
            digest.update((path.len() as u64).to_le_bytes());
            digest.update(path.as_bytes());
        }
        digest.finalize().into()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct StackSlot {
    base: Register,
    offset: u64,
}

#[derive(Clone, Copy, Debug)]
struct CallSite {
    index: usize,
    direct: Option<u32>,
    rcx: Option<StackSlot>,
    rdx: Option<StackSlot>,
}

struct BuilderPlan {
    start: usize,
    last_push: usize,
    vec_slot: StackSlot,
    push_callee: u32,
}

struct BranchFlow {
    guarded: BTreeSet<usize>,
    merges: BTreeMap<usize, Vec<Register>>,
}

struct ClassifiedCalls {
    assign_callee: Option<u32>,
    alloc_callee: Option<u32>,
    release_callees: BTreeSet<u32>,
}

fn slot_address(slot: StackSlot) -> io::Result<u64> {
    let base = match slot.base {
        Register::RSP => STACK,
        Register::RBP => FRAME,
        _ => return Err(invalid("cursor vector is not addressed from RSP or RBP")),
    };
    Ok(base.wrapping_add(slot.offset))
}

fn track_stack_slot(slots: &mut BTreeMap<Register, StackSlot>, i: &Instruction) {
    match i.mnemonic() {
        Mnemonic::Lea
            if i.op0_kind() == OpKind::Register
                && i.op0_register().size() <= 8
                && matches!(i.memory_base(), Register::RSP | Register::RBP)
                && i.memory_index() == Register::None =>
        {
            slots.insert(
                i.op0_register().full_register(),
                StackSlot {
                    base: i.memory_base(),
                    offset: i.memory_displacement64(),
                },
            );
        }
        Mnemonic::Mov
            if i.op0_kind() == OpKind::Register
                && i.op1_kind() == OpKind::Register
                && i.op0_register().size() <= 8
                && i.op1_register().size() <= 8 =>
        {
            let dst = i.op0_register().full_register();
            let src = i.op1_register().full_register();
            if let Some(&slot) = slots.get(&src) {
                slots.insert(dst, slot);
            } else {
                slots.remove(&dst);
            }
        }
        Mnemonic::Cmp | Mnemonic::Test => {}
        _ => {
            if i.op0_kind() == OpKind::Register && i.op0_register().size() <= 8 {
                slots.remove(&i.op0_register().full_register());
            }
        }
    }
}

fn clobber_slot_volatiles(slots: &mut BTreeMap<Register, StackSlot>) {
    for register in [
        Register::RAX,
        Register::RCX,
        Register::RDX,
        Register::R8,
        Register::R9,
        Register::R10,
        Register::R11,
    ] {
        slots.remove(&register);
    }
}

fn locate_builder(image: &PeImage, after: &[Instruction]) -> io::Result<BuilderPlan> {
    let mut slots = BTreeMap::new();
    let mut calls = Vec::new();
    let mut pushed_vectors = BTreeSet::new();
    let mut loaded: Option<(usize, StackSlot)> = None;

    for (index, i) in after.iter().enumerate() {
        if i.mnemonic() == Mnemonic::Call {
            let direct = call_target(image, i);
            let rcx = slots.get(&Register::RCX).copied();
            let rdx = slots.get(&Register::RDX).copied();
            if let Some(vec) = rdx
                && rcx != Some(vec)
                && pushed_vectors.contains(&vec)
            {
                loaded = Some((index, vec));
                break;
            }
            if direct.is_some()
                && let (Some(vec), Some(temp)) = (rcx, rdx)
                && vec != temp
            {
                pushed_vectors.insert(vec);
            }
            calls.push(CallSite {
                index,
                direct,
                rcx,
                rdx,
            });
            clobber_slot_volatiles(&mut slots);
        } else {
            track_stack_slot(&mut slots, i);
        }
    }

    let (_, vec_slot) = loaded.ok_or_else(|| invalid("cursor list load call not found"))?;
    let pushes: Vec<CallSite> = calls
        .iter()
        .copied()
        .filter(|site| site.rcx == Some(vec_slot) && site.rdx.is_some_and(|temp| temp != vec_slot))
        .collect();
    if pushes.is_empty() {
        return Err(invalid("cursor list has no push calls"));
    }
    let push_targets: BTreeSet<u32> = pushes
        .iter()
        .map(|site| site.direct.ok_or_else(|| invalid("indirect cursor push call")))
        .collect::<io::Result<_>>()?;
    let push_callee = sole(push_targets, "cursor vector push function")?;
    let first_push = pushes[0].index;
    let last_push = pushes[pushes.len() - 1].index;
    let temp_slots: BTreeSet<StackSlot> = pushes.iter().filter_map(|site| site.rdx).collect();

    let start = if let Some(init) = calls
        .iter()
        .rev()
        .find(|site| site.index < first_push && site.rcx == Some(vec_slot))
    {
        init.index + 1
    } else if let Some(prior) = calls.iter().rev().find(|site| {
        site.index < first_push
            && site
                .rcx
                .is_some_and(|slot| slot != vec_slot && !temp_slots.contains(&slot))
    }) {
        prior.index + 1
    } else {
        0
    };

    Ok(BuilderPlan {
        start,
        last_push,
        vec_slot,
        push_callee,
    })
}

fn validate_branches(
    image: &PeImage,
    builder: &[Instruction],
    push_callee: u32,
) -> io::Result<BranchFlow> {
    let mut guarded = BTreeSet::new();
    let mut merges: BTreeMap<usize, Vec<Register>> = BTreeMap::new();
    let last = builder.last().map_or(0, Instruction::ip);

    for (at, instruction) in builder.iter().enumerate() {
        match instruction.flow_control() {
            FlowControl::Return | FlowControl::IndirectBranch | FlowControl::UnconditionalBranch => {
                return Err(invalid("unexpected branch or return inside cursor list"));
            }
            FlowControl::IndirectCall => {
                return Err(invalid("indirect call inside cursor list"));
            }
            FlowControl::ConditionalBranch => {
                let target = instruction.near_branch_target();
                if target > last && builds_nothing(image, target)? {
                    continue;
                }
                let join = builder
                    .iter()
                    .position(|candidate| candidate.ip() == target)
                    .filter(|&join| join > at)
                    .ok_or_else(|| invalid("cursor list branch leaves builder or runs backwards"))?;
                for (offset, skipped) in builder[at + 1..join].iter().enumerate() {
                    let index = at + 1 + offset;
                    guarded.insert(index);
                    if skipped.mnemonic() == Mnemonic::Call {
                        let Some(callee) = call_target(image, skipped) else {
                            return Err(invalid("indirect call inside cursor release branch"));
                        };
                        if callee == push_callee {
                            return Err(invalid("branch skips a cursor push"));
                        }
                    }
                    let writes_memory = skipped.op0_kind() == OpKind::Memory
                        && !matches!(skipped.mnemonic(), Mnemonic::Cmp | Mnemonic::Test);
                    if writes_memory {
                        return Err(invalid("branch skips a memory write in cursor list"));
                    }
                    if skipped.op0_kind() == OpKind::Register
                        && !matches!(skipped.mnemonic(), Mnemonic::Cmp | Mnemonic::Test)
                    {
                        let register = skipped.op0_register();
                        let written = if register.size() > 8 {
                            register
                        } else {
                            register.full_register()
                        };
                        merges.entry(join).or_default().push(written);
                    }
                }
            }
            _ => {}
        }
    }

    Ok(BranchFlow { guarded, merges })
}

fn classify_calls(
    image: &PeImage,
    builder: &[Instruction],
    vec_slot: StackSlot,
    push_callee: u32,
    guarded: &BTreeSet<usize>,
) -> io::Result<ClassifiedCalls> {
    let mut release_callees = BTreeSet::new();
    let mut unguarded_callees = BTreeSet::new();
    let mut assign_candidates = BTreeSet::new();
    let mut alloc_candidates = BTreeSet::new();

    let mut slots = BTreeMap::new();
    let mut rdx_rdata = false;
    let mut r8_imm = false;
    let mut rcx_imm = false;

    for (index, i) in builder.iter().enumerate() {
        if i.mnemonic() == Mnemonic::Call {
            let callee = call_target(image, i)
                .ok_or_else(|| invalid("indirect call inside cursor list"))?;
            if guarded.contains(&index) {
                release_callees.insert(callee);
            } else {
                unguarded_callees.insert(callee);
                if callee != push_callee {
                    let rcx = slots.get(&Register::RCX).copied();
                    if rcx.is_some_and(|slot| slot != vec_slot) && rdx_rdata && r8_imm {
                        assign_candidates.insert(callee);
                    } else if rcx.is_none() && rcx_imm {
                        alloc_candidates.insert(callee);
                    } else {
                        return Err(invalid("unexpected call inside cursor list"));
                    }
                }
            }
            clobber_slot_volatiles(&mut slots);
            rdx_rdata = false;
            r8_imm = false;
            rcx_imm = false;
            continue;
        }

        track_stack_slot(&mut slots, i);
        if i.op0_kind() == OpKind::Register
            && !matches!(i.mnemonic(), Mnemonic::Cmp | Mnemonic::Test)
        {
            match i.op0_register().full_register() {
                Register::RDX => {
                    rdx_rdata = i.mnemonic() == Mnemonic::Lea
                        && rip_target(image, i)
                            .is_some_and(|target| image.is_read_only(target, 1));
                }
                Register::R8 => {
                    r8_imm = i.mnemonic() == Mnemonic::Mov && immediate(i, 1).is_ok();
                }
                Register::RCX => {
                    rcx_imm = i.mnemonic() == Mnemonic::Mov && immediate(i, 1).is_ok();
                }
                _ => {}
            }
        }
    }

    if release_callees.intersection(&unguarded_callees).next().is_some() {
        return Err(invalid("cursor release function is also called unconditionally"));
    }
    if assign_candidates.intersection(&alloc_candidates).next().is_some() {
        return Err(invalid("cursor string assign and allocator functions collide"));
    }
    if assign_candidates.len() > 1 {
        return Err(invalid("several cursor string assign functions found"));
    }
    if alloc_candidates.len() > 1 {
        return Err(invalid("several cursor string allocator functions found"));
    }

    Ok(ClassifiedCalls {
        assign_callee: assign_candidates.into_iter().next(),
        alloc_callee: alloc_candidates.into_iter().next(),
        release_callees,
    })
}

struct CursorConstruction<'a> {
    image: &'a PeImage,
    vec_addr: u64,
    push_callee: u32,
    assign_callee: Option<u32>,
    alloc_callee: Option<u32>,
    release_callees: BTreeSet<u32>,
    regs: Registers,
    vectors: BTreeMap<Register, Vec<u8>>,
    bytes: BTreeMap<u64, u8>,
    strings: BTreeMap<u64, String>,
    next_heap: u64,
    paths: Vec<String>,
}

impl<'a> CursorConstruction<'a> {
    fn new(
        image: &'a PeImage,
        vec_addr: u64,
        push_callee: u32,
        assign_callee: Option<u32>,
        alloc_callee: Option<u32>,
        release_callees: BTreeSet<u32>,
    ) -> Self {
        Self {
            image,
            vec_addr,
            push_callee,
            assign_callee,
            alloc_callee,
            release_callees,
            regs: Registers::known(&[(Register::RSP, STACK), (Register::RBP, FRAME)]),
            vectors: BTreeMap::new(),
            bytes: BTreeMap::new(),
            strings: BTreeMap::new(),
            next_heap: HEAP,
            paths: Vec::new(),
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
                    .ok_or_else(|| invalid("cursor construction address overflow"))?;
                self.bytes
                    .get(&address)
                    .copied()
                    .ok_or_else(|| invalid("cursor construction byte is unknown"))
            })
            .collect()
    }

    fn number_at(&self, address: u64, length: usize) -> io::Result<u64> {
        let bytes = self.bytes_at(address, length)?;
        let mut value = [0; 8];
        value[..length].copy_from_slice(&bytes);
        Ok(u64::from_le_bytes(value))
    }

    fn overwrite_strings(&mut self, address: u64, length: usize) {
        let end = address.saturating_add(length as u64);
        self.strings.retain(|&object, _| {
            object.saturating_add(STRING_OBJECT as u64) <= address || end <= object
        });
    }

    fn store(&mut self, address: u64, bytes: &[u8]) -> io::Result<()> {
        if address >= self.image.base {
            return Err(invalid("cursor construction writes image storage"));
        }
        self.overwrite_strings(address, bytes.len());
        for (offset, &byte) in bytes.iter().enumerate() {
            let address = address
                .checked_add(offset as u64)
                .ok_or_else(|| invalid("cursor construction address overflow"))?;
            self.bytes.insert(address, byte);
        }
        Ok(())
    }

    fn forget(&mut self, address: u64, length: usize) -> io::Result<()> {
        if address >= self.image.base {
            return Err(invalid("cursor construction writes image storage"));
        }
        self.overwrite_strings(address, length);
        for offset in 0..length as u64 {
            self.bytes.remove(&address.wrapping_add(offset));
        }
        Ok(())
    }

    fn string(&self, object: u64) -> io::Result<String> {
        if let Some(value) = self.strings.get(&object) {
            return Ok(value.clone());
        }
        let length = usize::try_from(self.number_at(object + 16, 8)?)
            .map_err(|_| invalid("cursor string length overflow"))?;
        let capacity = self.number_at(object + 24, 8)?;
        if length as u64 > capacity {
            return Err(invalid("cursor string length exceeds capacity"));
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
                .ok_or_else(|| invalid("cursor string extent overflow"))?,
        )?;
        if bytes.last() != Some(&0) || bytes[..length].contains(&0) {
            return Err(invalid("cursor string length or terminator differs"));
        }
        String::from_utf8(bytes[..length].to_vec())
            .map_err(|_| invalid("cursor path is not UTF-8"))
    }

    fn source_bytes(&self, i: &Instruction) -> io::Result<Vec<u8>> {
        let width = if i.op0_kind() == OpKind::Memory {
            i.memory_size().size()
        } else {
            i.op0_register().size()
        };
        if width == 0 || width > 16 {
            return Err(invalid("unsupported cursor construction width"));
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
                .ok_or_else(|| invalid("cursor construction vector is unknown"))?,
            OpKind::Register => register_value(&self.regs, i.op1_register())?
                .to_le_bytes()
                .to_vec(),
            _ => immediate(i, 1)?.to_le_bytes().to_vec(),
        };
        if matches!(i.mnemonic(), Mnemonic::Movsx | Mnemonic::Movsxd) && source_width <= 8 {
            let mut scalar = [0; 8];
            let copy = bytes.len().min(source_width);
            scalar[..copy].copy_from_slice(&bytes[..copy]);
            let extended = signed(u64::from_le_bytes(scalar), source_width) as u64;
            bytes = extended.to_le_bytes().to_vec();
        }
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
        if target == self.push_callee {
            if register_value(&self.regs, Register::RCX)? != self.vec_addr {
                return Err(invalid("cursor push targets another vector"));
            }
            let text = self.string(register_value(&self.regs, Register::RDX)?)?;
            self.paths.push(text);
        } else if self.assign_callee == Some(target) {
            let object = register_value(&self.regs, Register::RCX)?;
            let address = register_value(&self.regs, Register::RDX)?;
            let length = usize::try_from(register_value(&self.regs, Register::R8)?)
                .map_err(|_| invalid("cursor string length overflow"))?;
            let bytes = self.bytes_at(address, length)?;
            if bytes.contains(&0) {
                return Err(invalid("assigned cursor path contains a terminator"));
            }
            let text = String::from_utf8(bytes)
                .map_err(|_| invalid("assigned cursor path is not UTF-8"))?;
            self.forget(object, STRING_OBJECT)?;
            self.strings.insert(object, text);
        } else if self.alloc_callee == Some(target) {
            let length = register_value(&self.regs, Register::RCX)?;
            if length == 0 || length > MAX_ALLOCATION {
                return Err(invalid("cursor string allocation size is out of bounds"));
            }
            let address = self.next_heap;
            let step = length
                .checked_add(31)
                .map(|value| value & !15)
                .ok_or_else(|| invalid("cursor allocation extent overflow"))?;
            self.next_heap = address
                .checked_add(step)
                .ok_or_else(|| invalid("cursor allocation extent overflow"))?;
            if self.next_heap >= self.image.base {
                return Err(invalid("cursor allocation overlaps image"));
            }
            allocated = Some(address);
        } else if self.release_callees.contains(&target) {
        } else {
            return Err(invalid("unsupported call in cursor list"));
        }
        self.clobber_call();
        if let Some(address) = allocated {
            set_register(&mut self.regs, Register::RAX, address)?;
        }
        Ok(())
    }

    fn arithmetic(&mut self, i: &Instruction) -> io::Result<()> {
        if i.op0_kind() != OpKind::Register {
            return Err(invalid("unsupported arithmetic destination in cursor list"));
        }
        let register = i.op0_register();
        if matches!(register.full_register(), Register::RSP | Register::RBP) {
            return Err(invalid("stack or frame pointer moves inside cursor list"));
        }
        let width = register.size();
        if width == 0 || width > 8 {
            return Err(invalid("unsupported arithmetic register width in cursor list"));
        }
        let operand = if i.mnemonic() == Mnemonic::Inc {
            Ok(1)
        } else {
            match i.op1_kind() {
                OpKind::Register => register_value(&self.regs, i.op1_register()),
                OpKind::Memory => memory_address(&self.regs, i)
                    .and_then(|address| self.number_at(address, width)),
                _ => immediate(i, 1),
            }
        };
        match (register_value(&self.regs, register), operand) {
            (Ok(value), Ok(operand)) => {
                let next = if i.mnemonic() == Mnemonic::Sub {
                    value.wrapping_sub(operand)
                } else {
                    value.wrapping_add(operand)
                };
                set_register(&mut self.regs, register, next)?;
            }
            _ => self.regs.remove(&register),
        }
        Ok(())
    }

    fn instruction(&mut self, i: &Instruction) -> io::Result<()> {
        match i.mnemonic() {
            Mnemonic::Mov
            | Mnemonic::Movzx
            | Mnemonic::Movsx
            | Mnemonic::Movsxd
            | Mnemonic::Movups
            | Mnemonic::Movaps
            | Mnemonic::Movdqa
            | Mnemonic::Movdqu
            | Mnemonic::Movsd
            | Mnemonic::Movq => {
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
                        if matches!(
                            i.op0_register().full_register(),
                            Register::RSP | Register::RBP
                        ) {
                            return Err(invalid("stack or frame pointer overwritten in cursor list"));
                        }
                        if let Ok(bytes) = value {
                            let mut scalar = [0; 8];
                            scalar[..bytes.len()].copy_from_slice(&bytes);
                            set_register(
                                &mut self.regs,
                                i.op0_register(),
                                u64::from_le_bytes(scalar),
                            )?;
                        } else {
                            self.regs.remove(&i.op0_register());
                        }
                    }
                    OpKind::Memory => {
                        let address = memory_address(&self.regs, i)
                            .map_err(|_| invalid("cursor construction store address is unknown"))?;
                        match value {
                            Ok(bytes) => self.store(address, &bytes)?,
                            Err(_) => self.forget(address, i.memory_size().size())?,
                        }
                    }
                    _ => return Err(invalid("unsupported move destination in cursor list")),
                }
            }
            Mnemonic::Lea => {
                if matches!(
                    i.op0_register().full_register(),
                    Register::RSP | Register::RBP
                ) {
                    return Err(invalid("stack or frame pointer overwritten in cursor list"));
                }
                if let Ok(address) = memory_address(&self.regs, i) {
                    set_register(&mut self.regs, i.op0_register(), address)?;
                } else {
                    self.regs.remove(&i.op0_register());
                }
            }
            Mnemonic::Xor if i.op0_kind() == OpKind::Register && i.op0_register() == i.op1_register() => {
                set_register(&mut self.regs, i.op0_register(), 0)?;
            }
            Mnemonic::Xorps | Mnemonic::Xorpd | Mnemonic::Pxor
                if i.op0_kind() == OpKind::Register && i.op0_register() == i.op1_register() =>
            {
                self.vectors.insert(i.op0_register(), vec![0; 16]);
            }
            Mnemonic::Call => {
                let target = call_target(self.image, i)
                    .ok_or_else(|| invalid("indirect call inside cursor list"))?;
                self.call(target)?;
            }
            Mnemonic::Add | Mnemonic::Sub | Mnemonic::Inc => self.arithmetic(i)?,
            Mnemonic::Cmp
            | Mnemonic::Test
            | Mnemonic::Jbe
            | Mnemonic::Jb
            | Mnemonic::Ja
            | Mnemonic::Jae
            | Mnemonic::Je
            | Mnemonic::Jne
            | Mnemonic::Nop => {}
            _ => {
                return Err(invalid(format!(
                    "unsupported instruction {:?} in cursor list",
                    i.mnemonic()
                )));
            }
        }
        Ok(())
    }
}

fn call_target(image: &PeImage, i: &Instruction) -> Option<u32> {
    (i.mnemonic() == Mnemonic::Call
        && matches!(i.op0_kind(), OpKind::NearBranch64 | OpKind::NearBranch32))
        .then(|| image.va_to_rva(i.near_branch_target()).ok())
        .flatten()
}

fn rip_target(image: &PeImage, i: &Instruction) -> Option<u32> {
    i.is_ip_rel_memory_operand()
        .then(|| image.va_to_rva(i.ip_rel_memory_address()).ok())
        .flatten()
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
        Err(invalid("immediate operand is missing"))
    }
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

#[derive(Default)]
struct Registers(Vec<(Register, u64, u64)>);

impl Registers {
    fn known(registers: &[(Register, u64)]) -> Self {
        Self(
            registers
                .iter()
                .map(|&(register, value)| (register, value, u64::MAX))
                .collect(),
        )
    }

    fn value(&self, register: Register) -> Option<u64> {
        let (shift, width) = placement(register);
        let bits = mask(width) << shift;
        self.0
            .iter()
            .find(|(full, ..)| *full == register.full_register())
            .filter(|&&(_, _, known)| known & bits == bits)
            .map(|&(_, value, _)| (value >> shift) & mask(width))
    }

    fn set(&mut self, register: Register, value: u64) {
        let (shift, width) = placement(register);
        let full = register.full_register();
        let (value, known) = if width >= 4 {
            (value & mask(width), u64::MAX)
        } else {
            let (previous, known) = self
                .0
                .iter()
                .find(|(entry, ..)| *entry == full)
                .map_or((0, 0), |&(_, value, known)| (value, known));
            let bits = mask(width) << shift;
            ((previous & !bits) | ((value & mask(width)) << shift), known | bits)
        };
        match self.0.iter_mut().find(|(entry, ..)| *entry == full) {
            Some(entry) => *entry = (full, value, known),
            None => self.0.push((full, value, known)),
        }
    }

    fn remove(&mut self, register: &Register) {
        let full = register.full_register();
        self.0.retain(|(entry, ..)| *entry != full);
    }
}

fn placement(register: Register) -> (u32, usize) {
    let shift = if matches!(
        register,
        Register::AH | Register::BH | Register::CH | Register::DH
    ) {
        8
    } else {
        0
    };
    (shift, register.size())
}

fn register_value(regs: &Registers, register: Register) -> io::Result<u64> {
    regs.value(register)
        .ok_or_else(|| invalid("cursor construction register value is unknown"))
}

fn set_register(regs: &mut Registers, register: Register, value: u64) -> io::Result<()> {
    let width = register.size();
    if width == 0 || width > 8 {
        return Err(invalid("cursor construction unsupported register width"));
    }
    regs.set(register, value);
    Ok(())
}

fn memory_address(regs: &Registers, i: &Instruction) -> io::Result<u64> {
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
            .wrapping_mul(u64::from(i.memory_index_scale()))
    };
    Ok(base
        .wrapping_add(index)
        .wrapping_add(i.memory_displacement64()))
}

fn builds_nothing(image: &PeImage, va: u64) -> io::Result<bool> {
    let start = image.va_to_rva(va)?;
    let (fn_start, fn_end) = match image.function(start) {
        Ok(extent) => extent,
        Err(_) => return Ok(false),
    };
    let bytes = image.bytes(start, (fn_end - start).min(64) as usize)?;
    let mut decoder = Decoder::with_ip(64, bytes, va, DecoderOptions::NONE);
    for _ in 0..16 {
        if !decoder.can_decode() {
            break;
        }
        let instruction = decoder.decode();
        match instruction.mnemonic() {
            Mnemonic::Ret => return Ok(true),
            Mnemonic::Jmp if instruction.near_branch_target() == instruction.ip() => return Ok(true),
            Mnemonic::Call
                if matches!(
                    instruction.op0_kind(),
                    OpKind::NearBranch64 | OpKind::NearBranch32
                ) =>
            {
                return never_returns(image, image.va_to_rva(instruction.near_branch_target())?);
            }
            Mnemonic::Pop | Mnemonic::Add | Mnemonic::Lea | Mnemonic::Mov
                if instruction.op0_kind() == OpKind::Register => {}
            _ => return Ok(false),
        }
    }
    Ok(fn_start == start && never_returns(image, start)?)
}

fn never_returns(image: &PeImage, rva: u32) -> io::Result<bool> {
    let (start, end) = image.function(rva)?;
    if start != rva {
        return Ok(false);
    }
    for instruction in image.instructions(start, end)? {
        let leaves = match instruction.flow_control() {
            FlowControl::Return | FlowControl::IndirectBranch => true,
            FlowControl::UnconditionalBranch | FlowControl::ConditionalBranch => image
                .va_to_rva(instruction.near_branch_target())
                .is_ok_and(|target| !(start..end).contains(&target)),
            _ => false,
        };
        if leaves {
            return Ok(false);
        }
    }
    Ok(true)
}

fn sole<T>(candidates: impl IntoIterator<Item = T>, what: &str) -> io::Result<T> {
    let mut candidates = candidates.into_iter();
    match (candidates.next(), candidates.next()) {
        (Some(found), None) => Ok(found),
        (None, _) => Err(invalid(format!("no {what} found"))),
        (Some(_), Some(_)) => Err(invalid(format!("several candidates for the {what}"))),
    }
}

fn path_stem(path: &str) -> Option<&str> {
    let file = path.rsplit(['/', '\\']).next()?;
    let (stem, extension) = file.rsplit_once('.')?;
    (!stem.is_empty() && !extension.is_empty()).then_some(stem)
}

fn validate_path(path: &str) -> io::Result<()> {
    if path.is_empty()
        || !path
            .bytes()
            .all(|byte| byte.is_ascii_graphic() || byte == b' ')
        || path.starts_with(['/', '\\'])
        || path
            .split(['/', '\\'])
            .any(|segment| segment.is_empty() || segment == "." || segment == "..")
        || path_stem(path).is_none()
    {
        return Err(invalid(format!(
            "cursor path {path:?} is not a valid relative cursor file path"
        )));
    }
    Ok(())
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executable_tables::pe::fnv1a;
    use crate::executable_tables::synthetic_image::{Code, RDATA, SyntheticImage, TEXT};

    const SYNTHETIC_ANCHOR_TEXT: &str = "synthetic_cursor_log";
    const MAIN_FN: u32 = TEXT;
    const INIT_FN: u32 = TEXT + 0x500;
    const ASSIGN_FN: u32 = TEXT + 0x520;
    const PUSH_FN: u32 = TEXT + 0x540;
    const RELEASE_FN: u32 = TEXT + 0x560;
    const ALLOC_FN: u32 = TEXT + 0x580;
    const DELETE_FN: u32 = TEXT + 0x5a0;
    const MOUSE_FN: u32 = TEXT + 0x5c0;
    const ABORT_FN: u32 = TEXT + 0x5e0;

    fn synthetic_anchor() -> Anchor {
        Anchor {
            what: "the synthetic cursor log message",
            text: Hashed {
                length: SYNTHETIC_ANCHOR_TEXT.len(),
                fnv: fnv1a(SYNTHETIC_ANCHOR_TEXT.as_bytes()),
            },
        }
    }

    fn emit_assign_push(code: Code, string_rva: u32, len: u32, with_release_guard: bool) -> Code {
        let mut code = code
            .raw(&[0x0f, 0x57, 0xc0])
            .raw(&[0x0f, 0x11, 0x44, 0x24, 0x40])
            .raw(&[0xf3, 0x0f, 0x7f, 0x44, 0x24, 0x50])
            .raw(&[0x41, 0xb8])
            .raw(&len.to_le_bytes())
            .rip(&[0x48, 0x8d, 0x15], string_rva)
            .raw(&[0x48, 0x8d, 0x4c, 0x24, 0x40])
            .call(ASSIGN_FN)
            .raw(&[0x48, 0x8d, 0x54, 0x24, 0x40])
            .raw(&[0x48, 0x8d, 0x4c, 0x24, 0x60])
            .call(PUSH_FN);
        if with_release_guard {
            code = code
                .raw(&[0x4c, 0x8b, 0x44, 0x24, 0x58])
                .raw(&[0x49, 0x83, 0xf8, 0x0f])
                .raw(&[0x76, 0x0f])
                .raw(&[0x48, 0x8b, 0x54, 0x24, 0x40])
                .raw(&[0x48, 0x8d, 0x4c, 0x24, 0x40])
                .call(RELEASE_FN);
        }
        code
    }

    fn emit_inlined_23_byte_push(code: Code, meta_rva: u32, string_rva: u32) -> Code {
        code.raw(&[0x48, 0x89, 0x74, 0x24, 0x48])
            .raw(&[0xb9, 0x20, 0x00, 0x00, 0x00])
            .call(ALLOC_FN)
            .raw(&[0x48, 0x8b, 0xd0])
            .raw(&[0x48, 0x89, 0x44, 0x24, 0x40])
            .rip(&[0x66, 0x0f, 0x6f, 0x05], meta_rva)
            .raw(&[0xf3, 0x0f, 0x7f, 0x44, 0x24, 0x50])
            .rip(&[0x0f, 0x10, 0x05], string_rva)
            .raw(&[0x0f, 0x11, 0x00])
            .rip(&[0x8b, 0x0d], string_rva + 16)
            .raw(&[0x89, 0x48, 0x10])
            .rip(&[0x0f, 0xb7, 0x0d], string_rva + 20)
            .raw(&[0x66, 0x89, 0x48, 0x14])
            .rip(&[0x0f, 0xb6, 0x05], string_rva + 22)
            .raw(&[0x88, 0x42, 0x16])
            .raw(&[0xc6, 0x42, 0x17, 0x00])
            .raw(&[0x48, 0x8d, 0x54, 0x24, 0x40])
            .raw(&[0x48, 0x8d, 0x4c, 0x24, 0x60])
            .call(PUSH_FN)
            .raw(&[0x48, 0x8b, 0x54, 0x24, 0x58])
            .raw(&[0x48, 0x83, 0xfa, 0x0f])
            .raw(&[0x76, 0x32])
            .raw(&[0x48, 0xff, 0xc2])
            .raw(&[0x48, 0x8b, 0x4c, 0x24, 0x40])
            .raw(&[0x48, 0x8b, 0xc1])
            .raw(&[0x48, 0x81, 0xfa, 0x00, 0x10, 0x00, 0x00])
            .raw(&[0x72, 0x19])
            .raw(&[0x48, 0x83, 0xc2, 0x27])
            .raw(&[0x48, 0x8b, 0x49, 0xf8])
            .raw(&[0x48, 0x2b, 0xc1])
            .raw(&[0x48, 0x83, 0xc0, 0xf8])
            .raw(&[0x48, 0x83, 0xf8, 0x1f])
            .rip(&[0x0f, 0x87], ABORT_FN)
            .call(DELETE_FN)
    }

    fn synthetic_cursor_image(include_inlined: bool, extra_call: Option<u32>) -> PeImage {
        let mut image = SyntheticImage::new();
        for helper in [INIT_FN, ASSIGN_FN, PUSH_FN, RELEASE_FN, ALLOC_FN, DELETE_FN, MOUSE_FN] {
            image.function(Code::at(helper).ret());
        }
        image.function(Code::at(ABORT_FN).raw(&[0xeb, 0xfe]));

        let anchor_rva = RDATA;
        let first_rva = RDATA + 0x40;
        let second_rva = RDATA + 0x80;
        let third_rva = RDATA + 0xc0;
        let meta_rva = RDATA + 0x100;

        let first = "gfx/cursors/alpha.cur";
        let second = "gfx/cursors/beta_01.ani";
        let third = "gfx/alt/gamma.cur";
        assert_eq!(second.len(), 23);

        image.text(anchor_rva, SYNTHETIC_ANCHOR_TEXT);
        image.text(first_rva, first);
        image.text(second_rva, second);
        image.text(third_rva, third);
        let mut meta = [0u8; 16];
        meta[0..8].copy_from_slice(&(second.len() as u64).to_le_bytes());
        meta[8..16].copy_from_slice(&31_u64.to_le_bytes());
        image.put(meta_rva, &meta);

        let mut main = Code::at(MAIN_FN)
            .rip(&[0x48, 0x8d, 0x15], anchor_rva)
            .raw(&[0x48, 0x8d, 0x4c, 0x24, 0x60])
            .call(INIT_FN);
        main = emit_assign_push(main, first_rva, first.len() as u32, true);
        if include_inlined {
            main = emit_inlined_23_byte_push(main, meta_rva, second_rva);
        }
        if let Some(callee) = extra_call {
            main = main.call(callee);
        }
        main = emit_assign_push(main, third_rva, third.len() as u32, true);
        main = emit_assign_push(main, first_rva, first.len() as u32, true);
        main = main
            .call(MOUSE_FN)
            .raw(&[0x48, 0x8b, 0x08])
            .raw(&[0x4c, 0x8b, 0x41, 0x50])
            .raw(&[0x48, 0x8d, 0x54, 0x24, 0x60])
            .raw(&[0x48, 0x8b, 0xc8])
            .raw(&[0x41, 0xff, 0xd0])
            .ret();

        image.function(main);
        image.image()
    }

    #[test]
    fn decodes_assign_and_inlined_heap_cursor_pushes_in_order() {
        let image = synthetic_cursor_image(true, None);
        let table = CursorTable::decode_with_anchor(&image, synthetic_anchor()).unwrap();
        assert_eq!(
            table.paths,
            [
                "gfx/cursors/alpha.cur",
                "gfx/cursors/beta_01.ani",
                "gfx/alt/gamma.cur",
                "gfx/cursors/alpha.cur",
            ]
        );
        assert_eq!(table.path("alpha"), Ok("gfx/cursors/alpha.cur"));
        assert_eq!(table.path("beta_01"), Ok("gfx/cursors/beta_01.ani"));
        assert_eq!(table.path("gamma"), Ok("gfx/alt/gamma.cur"));
    }

    #[test]
    fn refuses_missing_anchor_or_unexpected_call_in_builder() {
        let empty = SyntheticImage::new()
            .function(Code::at(MAIN_FN).ret())
            .image();
        assert!(CursorTable::decode_with_anchor(&empty, synthetic_anchor()).is_err());

        let unexpected = synthetic_cursor_image(false, Some(MOUSE_FN));
        assert!(CursorTable::decode_with_anchor(&unexpected, synthetic_anchor()).is_err());
    }

    #[test]
    fn path_lookup_accepts_duplicate_identical_paths_and_refuses_ambiguous_or_absent_names() {
        let table = CursorTable::from_paths([
            "gfx/cursors/alpha.cur".to_owned(),
            "gfx/cursors/alpha.cur".to_owned(),
            "gfx/cursors/clash.cur".to_owned(),
            "gfx/alt/clash.ani".to_owned(),
        ])
        .unwrap();
        assert_eq!(table.path("alpha"), Ok("gfx/cursors/alpha.cur"));
        assert!(
            table
                .path("clash")
                .unwrap_err()
                .contains("maps to multiple paths")
        );
        assert!(
            table
                .path("missing")
                .unwrap_err()
                .contains("not in the recorded cursor registry")
        );
        for bad in ["", "/gfx/a.cur", "gfx/../a.cur", "gfx/no_ext", "gfx/.cur"] {
            assert!(CursorTable::from_paths([bad.to_owned()]).is_err(), "{bad:?}");
        }
    }
}
