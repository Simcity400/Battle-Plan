use super::pe::{Anchor, Hashed, PeImage};
use crate::build::ExecutableIdentity;
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

const LOAD_CURSOR_ERROR: Anchor = Anchor {
    what: "the cursor load error message",
    text: Hashed {
        length: 22,
        fnv: 0x8082_90e0_1e98_9a0f,
    },
};

const MOUSE_TYPE: Anchor = Anchor {
    what: "the mouse class type",
    text: Hashed {
        length: 15,
        fnv: 0x8b19_de52_0016_0231,
    },
};

const STACK: u64 = 0x10_0000;
const HEAP: u64 = 0x30_0000;
const MOUSE_OBJ: u64 = 0x50_0000;
const MOUSE_VTABLE: u64 = 0x50_1000;
const TLS_ARRAY: u64 = 0x60_0000;
const TLS_SLOT: u64 = 0x60_1000;
const SENTINEL_RET: u64 = 0x00ff_ffff_0000_0000;
const STRING_OBJECT: usize = 32;
const VECTOR_HEADER: usize = 24;
const MAX_ALLOCATION: u64 = 0x1_0000;
const MAX_HELPER_STEPS: usize = 2048;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum CursorAction {
    Normal = 0,
    DragSelect = 1,
    NoMove = 2,
    AttackMove = 3,
    AssignAir = 4,
    PingOffensive = 5,
    PingDefensive = 6,
    FriendlyMove = 7,
    MoveToEnemyProvince = 8,
    AddUnits = 9,
    ChainMove = 10,
    ChainMoveNaval = 11,
}

impl CursorAction {
    pub const ALL: [Self; 12] = [
        Self::Normal,
        Self::DragSelect,
        Self::NoMove,
        Self::AttackMove,
        Self::AssignAir,
        Self::PingOffensive,
        Self::PingDefensive,
        Self::FriendlyMove,
        Self::MoveToEnemyProvince,
        Self::AddUnits,
        Self::ChainMove,
        Self::ChainMoveNaval,
    ];

    pub const fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Normal),
            1 => Some(Self::DragSelect),
            2 => Some(Self::NoMove),
            3 => Some(Self::AttackMove),
            4 => Some(Self::AssignAir),
            5 => Some(Self::PingOffensive),
            6 => Some(Self::PingDefensive),
            7 => Some(Self::FriendlyMove),
            8 => Some(Self::MoveToEnemyProvince),
            9 => Some(Self::AddUnits),
            10 => Some(Self::ChainMove),
            11 => Some(Self::ChainMoveNaval),
            _ => None,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::DragSelect => "dragselect",
            Self::NoMove => "no_move",
            Self::AttackMove => "attack_move",
            Self::AssignAir => "assign_air",
            Self::PingOffensive => "ping_offensive",
            Self::PingDefensive => "ping_defensive",
            Self::FriendlyMove => "friendly_move",
            Self::MoveToEnemyProvince => "move_to_enemy_province",
            Self::AddUnits => "add_units",
            Self::ChainMove => "chain_move",
            Self::ChainMoveNaval => "chain_move_naval",
        }
    }
}

const PINNED_ACTION_SLOTS: [(CursorAction, u32); 12] = [
    (CursorAction::Normal, 0),
    (CursorAction::DragSelect, 3),
    (CursorAction::NoMove, 5),
    (CursorAction::FriendlyMove, 6),
    (CursorAction::AttackMove, 7),
    (CursorAction::MoveToEnemyProvince, 9),
    (CursorAction::AddUnits, 11),
    (CursorAction::ChainMove, 20),
    (CursorAction::ChainMoveNaval, 21),
    (CursorAction::AssignAir, 23),
    (CursorAction::PingOffensive, 24),
    (CursorAction::PingDefensive, 25),
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CursorSlot {
    Path(String),
    Unusable(String),
}

impl CursorSlot {
    pub fn validated(path: String) -> Self {
        if path.is_empty() {
            return Self::Unusable("empty cursor path".to_owned());
        }
        match validate_path(&path) {
            Ok(()) => Self::Path(path),
            Err(error) => Self::Unusable(error.to_string()),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CursorTable {
    pub slots: Vec<CursorSlot>,
    pub bindings: BTreeMap<CursorAction, u32>,
}

struct Anchors {
    loading_log: Anchor,
    load_error: Anchor,
    mouse_type: Anchor,
}

impl CursorTable {
    pub fn decode(image: &PeImage, identity: &ExecutableIdentity) -> io::Result<Self> {
        let anchors = Anchors {
            loading_log: LOADING_CURSOR_LOG,
            load_error: LOAD_CURSOR_ERROR,
            mouse_type: MOUSE_TYPE,
        };
        let bindings: Vec<(CursorAction, u32)> = if identity.recognised() {
            PINNED_ACTION_SLOTS.to_vec()
        } else {
            Vec::new()
        };
        Self::decode_with_anchors(image, anchors, &bindings)
    }

    fn decode_with_anchors(
        image: &PeImage,
        anchors: Anchors,
        action_bindings: &[(CursorAction, u32)],
    ) -> io::Result<Self> {
        let loader = verify_loader(image, anchors.mouse_type, anchors.load_error)?;
        for &(action, slot) in action_bindings {
            if slot as usize >= loader.capacity {
                return Err(invalid(format!(
                    "verified binding for {} targets slot {slot}, outside loader capacity {}",
                    action.name(),
                    loader.capacity
                )));
            }
        }

        let strings = image.find_hashed_strings(&[anchors.loading_log.text]);
        let references: Vec<u32> = image
            .references_to_each(&strings[0])
            .into_iter()
            .flatten()
            .collect();
        let anchor_rva = sole(references, &format!("reference to {}", anchors.loading_log.what))?;
        let (start, end) = image.function(anchor_rva)?;
        let instructions = image.instructions(start, end)?;
        let anchor_index = sole(
            instructions
                .iter()
                .enumerate()
                .filter(|(_, instruction)| {
                    rip_target(image, instruction)
                        .is_some_and(|target| strings[0].contains(&target))
                })
                .map(|(index, _)| index),
            &format!("instruction referencing {}", anchors.loading_log.what),
        )?;
        let frame_delta = derive_frame_delta(&instructions, anchor_index)?;
        let after = &instructions[anchor_index + 1..];

        let plan = locate_builder(image, after, &loader, frame_delta)?;
        let builder = &after[plan.start..plan.load_index];
        let load_inst = after[plan.load_index];
        let flow = validate_branches(image, builder, plan.push_callee)?;
        let calls = classify_calls(
            image,
            builder,
            plan.last_push - plan.start,
            plan.vec_slot,
            plan.push_callee,
            &flow.guarded,
        )?;

        if let Some(init_callee) = plan.init_callee {
            verify_init(image, init_callee)?;
        }
        verify_push(image, plan.push_callee)?;
        if let Some(alloc_callee) = calls.alloc_callee {
            verify_alloc(image, alloc_callee)?;
        }
        if let Some(assign_callee) = calls.assign_callee {
            verify_assign(image, assign_callee, calls.alloc_callee)?;
        }
        for (&release_callee, &takes_string_obj) in &calls.release_callees {
            verify_release(image, release_callee, takes_string_obj)?;
        }

        let vec_addr = slot_address(plan.vec_slot, frame_delta)?;
        let mut construction = CursorConstruction::new(
            image,
            frame_delta,
            vec_addr,
            plan.push_callee,
            calls.assign_callee,
            calls.alloc_callee,
            calls.mouse_getter,
            calls.release_callees.keys().copied().collect(),
            &loader,
        );
        for (index, instruction) in builder.iter().enumerate() {
            for &register in flow.merges.get(&index).into_iter().flatten() {
                construction.regs.remove(&register);
                construction.vectors.remove(&register);
            }
            construction.instruction(instruction)?;
        }
        verify_load_site(&construction, &load_inst, &loader)?;

        if construction.paths.is_empty() {
            return Err(invalid("cursor registry pushed no paths"));
        }
        Ok(Self::from_raw_paths(
            construction.paths,
            loader.capacity,
            action_bindings.iter().copied(),
        ))
    }

    pub fn from_slots(
        slots: Vec<CursorSlot>,
        bindings: impl IntoIterator<Item = (CursorAction, u32)>,
    ) -> Self {
        Self {
            slots,
            bindings: bindings.into_iter().collect(),
        }
    }

    pub fn from_raw_paths(
        raw: impl IntoIterator<Item = String>,
        capacity: usize,
        bindings: impl IntoIterator<Item = (CursorAction, u32)>,
    ) -> Self {
        let mut raw = raw.into_iter();
        let mut slots = Vec::with_capacity(capacity);
        for _ in 0..capacity {
            match raw.next() {
                Some(path) => slots.push(CursorSlot::validated(path)),
                None => slots.push(CursorSlot::Unusable(
                    "not populated in cursor registry".to_owned(),
                )),
            }
        }
        Self {
            slots,
            bindings: bindings.into_iter().collect(),
        }
    }

    pub fn path_for(&self, action: CursorAction) -> Result<&str, String> {
        let Some(&slot) = self.bindings.get(&action) else {
            return Err(format!(
                "{}: cursor slot binding is not verified for this build",
                action.name()
            ));
        };
        match self.slots.get(slot as usize) {
            Some(CursorSlot::Path(path)) => Ok(path.as_str()),
            Some(CursorSlot::Unusable(reason)) => {
                Err(format!("{} (slot {slot}): {reason}", action.name()))
            }
            None => Err(format!(
                "{} (slot {slot}): exceeds the recorded cursor registry ({} slots)",
                action.name(),
                self.slots.len()
            )),
        }
    }

    pub(super) fn content_digest(&self) -> [u8; 32] {
        let mut digest = Sha256::new();
        digest.update((self.slots.len() as u64).to_le_bytes());
        for slot in &self.slots {
            match slot {
                CursorSlot::Path(path) => {
                    digest.update([0]);
                    digest.update((path.len() as u64).to_le_bytes());
                    digest.update(path.as_bytes());
                }
                CursorSlot::Unusable(reason) => {
                    digest.update([1]);
                    digest.update((reason.len() as u64).to_le_bytes());
                    digest.update(reason.as_bytes());
                }
            }
        }
        digest.update((self.bindings.len() as u64).to_le_bytes());
        for (&action, &slot) in &self.bindings {
            digest.update([action as u8]);
            digest.update(slot.to_le_bytes());
        }
        digest.finalize().into()
    }
}

struct LoaderInfo {
    entry: u32,
    vtable_offset: u64,
    capacity: usize,
}

fn verify_loader(
    image: &PeImage,
    mouse_type: Anchor,
    load_error: Anchor,
) -> io::Result<LoaderInfo> {
    let vtable = sole(
        image
            .classes_named(mouse_type.text)
            .filter(|class| class.offset == 0)
            .map(|class| class.vtable),
        &format!("primary vtable of {}", mouse_type.what),
    )?;
    let strings = image.find_hashed_strings(&[load_error.text]);
    let references: Vec<u32> = image
        .references_to_each(&strings[0])
        .into_iter()
        .flatten()
        .collect();
    let loader_fn = sole(
        image.referencing_functions(references)?,
        &format!("function referencing {}", load_error.what),
    )?;
    let mut matching_slots = Vec::new();
    let mut slot_index = 0u64;
    let mut at = vtable;
    while let Ok(pointer) = image.u64(at)
        && image.is_code_pointer(pointer)
    {
        if image.va_to_rva(pointer).ok() == Some(loader_fn.0) {
            matching_slots.push(slot_index * 8);
        }
        slot_index += 1;
        at = at
            .checked_add(8)
            .ok_or_else(|| invalid("mouse vtable address overflows"))?;
    }
    let vtable_offset = sole(matching_slots, "mouse cursor loader vtable slot")?;

    let instructions = image.instructions(loader_fn.0, loader_fn.1)?;
    let loader_start_va = image.base + u64::from(loader_fn.0);
    let loader_end_va = image.base + u64::from(loader_fn.1);
    let mut bounds = Vec::new();
    for pair in instructions.windows(2) {
        let [cmp, branch] = pair else { continue };
        if cmp.mnemonic() == Mnemonic::Cmp
            && cmp.op0_kind() == OpKind::Register
            && cmp.op0_register().size() == 4
            && let Ok(bound) = immediate(cmp, 1)
            && bound > 0
            && bound <= 4096
            && matches!(branch.mnemonic(), Mnemonic::Jl | Mnemonic::Jb | Mnemonic::Jne)
            && matches!(
                branch.op0_kind(),
                OpKind::NearBranch16 | OpKind::NearBranch32 | OpKind::NearBranch64
            )
        {
            let target = branch.near_branch_target();
            if target >= loader_start_va && target < cmp.ip() && target < loader_end_va {
                bounds.push(bound as usize);
            }
        }
    }
    let capacity = sole(bounds, "cursor loader handle capacity")?;
    Ok(LoaderInfo {
        entry: loader_fn.0,
        vtable_offset,
        capacity,
    })
}

fn derive_frame_delta(instructions: &[Instruction], stop_at: usize) -> io::Result<Option<i64>> {
    let mut rsp: i64 = 0;
    let mut rbp: Option<i64> = None;
    for instruction in instructions.iter().take(stop_at) {
        match instruction.flow_control() {
            FlowControl::Next => {}
            _ => break,
        }
        match instruction.mnemonic() {
            Mnemonic::Push => rsp = rsp.wrapping_sub(8),
            Mnemonic::Pop => rsp = rsp.wrapping_add(8),
            Mnemonic::Sub
                if instruction.op0_kind() == OpKind::Register
                    && instruction.op0_register() == Register::RSP =>
            {
                let amount = immediate(instruction, 1)? as i64;
                rsp = rsp.wrapping_sub(amount);
            }
            Mnemonic::Add
                if instruction.op0_kind() == OpKind::Register
                    && instruction.op0_register() == Register::RSP =>
            {
                let amount = immediate(instruction, 1)? as i64;
                rsp = rsp.wrapping_add(amount);
            }
            Mnemonic::Lea
                if instruction.op0_kind() == OpKind::Register
                    && instruction.op0_register() == Register::RBP =>
            {
                if instruction.memory_base() == Register::RSP
                    && instruction.memory_index() == Register::None
                {
                    rbp = Some(rsp.wrapping_add(instruction.memory_displacement64() as i64));
                } else {
                    rbp = None;
                }
            }
            Mnemonic::Mov
                if instruction.op0_kind() == OpKind::Register
                    && instruction.op0_register() == Register::RBP =>
            {
                if instruction.op1_kind() == OpKind::Register
                    && instruction.op1_register() == Register::RSP
                {
                    rbp = Some(rsp);
                } else {
                    rbp = None;
                }
            }
            _ => {
                if instruction.op0_kind() == OpKind::Register
                    && !matches!(instruction.mnemonic(), Mnemonic::Cmp | Mnemonic::Test)
                {
                    match instruction.op0_register().full_register() {
                        Register::RSP => {
                            return Err(invalid("unsupported stack adjustment in function prologue"));
                        }
                        Register::RBP => rbp = None,
                        _ => {}
                    }
                }
            }
        }
    }
    Ok(rbp.map(|frame| frame.wrapping_sub(rsp)))
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
    r8: Option<StackSlot>,
    r9: Option<StackSlot>,
}

struct BuilderPlan {
    start: usize,
    last_push: usize,
    load_index: usize,
    vec_slot: StackSlot,
    init_callee: Option<u32>,
    push_callee: u32,
}

struct BranchFlow {
    guarded: BTreeSet<usize>,
    merges: BTreeMap<usize, Vec<Register>>,
}

struct ClassifiedCalls {
    assign_callee: Option<u32>,
    alloc_callee: Option<u32>,
    mouse_getter: Option<u32>,
    release_callees: BTreeMap<u32, bool>,
}

fn slot_address(slot: StackSlot, frame_delta: Option<i64>) -> io::Result<u64> {
    let base = match slot.base {
        Register::RSP => STACK,
        Register::RBP => STACK.wrapping_add(
            frame_delta.ok_or_else(|| invalid("cursor vector uses RBP without a proven frame delta"))?
                as u64,
        ),
        _ => return Err(invalid("cursor vector is not addressed from RSP or RBP")),
    };
    Ok(base.wrapping_add(slot.offset))
}

fn same_stack_slot(left: StackSlot, right: StackSlot, frame_delta: Option<i64>) -> bool {
    if left == right {
        return true;
    }
    match (slot_address(left, frame_delta), slot_address(right, frame_delta)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
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

fn clobber_vtable_volatiles(
    vtable_ptrs: &mut BTreeSet<Register>,
    vtable_slots: &mut BTreeMap<Register, u64>,
) {
    for register in [
        Register::RAX,
        Register::RCX,
        Register::RDX,
        Register::R8,
        Register::R9,
        Register::R10,
        Register::R11,
    ] {
        vtable_ptrs.remove(&register);
        vtable_slots.remove(&register);
    }
}

fn track_vtable(
    vtable_ptrs: &mut BTreeSet<Register>,
    vtable_slots: &mut BTreeMap<Register, u64>,
    i: &Instruction,
) {
    if i.op0_kind() != OpKind::Register || matches!(i.mnemonic(), Mnemonic::Cmp | Mnemonic::Test) {
        return;
    }
    let dst = i.op0_register().full_register();
    if i.mnemonic() == Mnemonic::Mov && i.op1_kind() == OpKind::Memory && i.memory_size().size() == 8 {
        if i.memory_index() == Register::None
            && !matches!(i.memory_base(), Register::None | Register::RSP | Register::RBP)
        {
            let base = i.memory_base().full_register();
            if vtable_ptrs.contains(&base) {
                vtable_ptrs.remove(&dst);
                vtable_slots.insert(dst, i.memory_displacement64());
                return;
            }
            if i.memory_displacement64() == 0 {
                vtable_slots.remove(&dst);
                vtable_ptrs.insert(dst);
                return;
            }
        }
    }
    vtable_ptrs.remove(&dst);
    vtable_slots.remove(&dst);
}

fn is_verified_load_call(
    image: &PeImage,
    i: &Instruction,
    loader: &LoaderInfo,
    vtable_ptrs: &BTreeSet<Register>,
    vtable_slots: &BTreeMap<Register, u64>,
) -> bool {
    if call_target(image, i) == Some(loader.entry) {
        return true;
    }
    if i.op0_kind() == OpKind::Register {
        return vtable_slots.get(&i.op0_register().full_register()) == Some(&loader.vtable_offset);
    }
    if i.op0_kind() == OpKind::Memory && i.memory_index() == Register::None {
        return vtable_ptrs.contains(&i.memory_base().full_register())
            && i.memory_displacement64() == loader.vtable_offset;
    }
    false
}

fn locate_builder(
    image: &PeImage,
    after: &[Instruction],
    loader: &LoaderInfo,
    frame_delta: Option<i64>,
) -> io::Result<BuilderPlan> {
    let mut slots = BTreeMap::new();
    let mut vtable_ptrs = BTreeSet::new();
    let mut vtable_slots = BTreeMap::new();
    let mut calls = Vec::new();
    let mut load_candidates = Vec::new();

    for (index, i) in after.iter().enumerate() {
        if i.mnemonic() == Mnemonic::Call {
            let direct = call_target(image, i);
            let rcx = slots.get(&Register::RCX).copied();
            let rdx = slots.get(&Register::RDX).copied();
            let r8 = slots.get(&Register::R8).copied();
            let r9 = slots.get(&Register::R9).copied();
            if is_verified_load_call(image, i, loader, &vtable_ptrs, &vtable_slots)
                && let Some(vec) = rdx
                && !rcx.is_some_and(|slot| same_stack_slot(slot, vec, frame_delta))
            {
                load_candidates.push((index, vec));
            }
            calls.push(CallSite {
                index,
                direct,
                rcx,
                rdx,
                r8,
                r9,
            });
            clobber_slot_volatiles(&mut slots);
            clobber_vtable_volatiles(&mut vtable_ptrs, &mut vtable_slots);
        } else {
            track_stack_slot(&mut slots, i);
            track_vtable(&mut vtable_ptrs, &mut vtable_slots, i);
        }
    }

    let (load_index, vec_slot) = sole(load_candidates, "verified cursor loader call")?;
    slot_address(vec_slot, frame_delta)?;

    let touches_vec = |site: &CallSite| {
        [site.rcx, site.rdx, site.r8, site.r9]
            .into_iter()
            .flatten()
            .any(|slot| same_stack_slot(slot, vec_slot, frame_delta))
    };

    let vec_calls: Vec<CallSite> = calls
        .iter()
        .copied()
        .filter(|site| site.index < load_index && touches_vec(site))
        .collect();
    if vec_calls.is_empty() {
        return Err(invalid("cursor list has no calls populating its vector"));
    }

    let mut init_site: Option<CallSite> = None;
    let mut pushes = Vec::new();
    for site in vec_calls {
        let rcx_is_vec = site
            .rcx
            .is_some_and(|slot| same_stack_slot(slot, vec_slot, frame_delta));
        let rdx_is_temp = site
            .rdx
            .is_some_and(|slot| !same_stack_slot(slot, vec_slot, frame_delta));
        let other_has_vec = [site.rdx, site.r8, site.r9]
            .into_iter()
            .flatten()
            .any(|slot| same_stack_slot(slot, vec_slot, frame_delta));
        if rcx_is_vec && rdx_is_temp && !other_has_vec {
            pushes.push(site);
        } else if pushes.is_empty()
            && init_site.is_none()
            && rcx_is_vec
            && site.rdx.is_none()
            && !other_has_vec
        {
            init_site = Some(site);
        } else {
            return Err(invalid(
                "unverified call touches cursor vector before cursor load",
            ));
        }
    }

    if pushes.is_empty() {
        return Err(invalid("cursor list has no push calls"));
    }
    let push_targets: BTreeSet<u32> = pushes
        .iter()
        .map(|site| site.direct.ok_or_else(|| invalid("indirect cursor push call")))
        .collect::<io::Result<_>>()?;
    let push_callee = sole(push_targets, "cursor vector push function")?;
    let init_callee = match init_site {
        Some(site) => Some(
            site.direct
                .ok_or_else(|| invalid("indirect cursor vector initializer call"))?,
        ),
        None => None,
    };
    let first_push = pushes[0].index;
    let last_push = pushes[pushes.len() - 1].index;
    let temp_slots: Vec<StackSlot> = pushes.iter().filter_map(|site| site.rdx).collect();

    let start = if let Some(init) = init_site {
        init.index + 1
    } else if let Some(prior) = calls.iter().rev().find(|site| {
        site.index < first_push
            && site.rcx.is_some_and(|slot| {
                !same_stack_slot(slot, vec_slot, frame_delta)
                    && !temp_slots
                        .iter()
                        .any(|&temp| same_stack_slot(slot, temp, frame_delta))
            })
    }) {
        prior.index + 1
    } else {
        0
    };

    Ok(BuilderPlan {
        start,
        last_push,
        load_index,
        vec_slot,
        init_callee,
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
    last_push_in_builder: usize,
    vec_slot: StackSlot,
    push_callee: u32,
    guarded: &BTreeSet<usize>,
) -> io::Result<ClassifiedCalls> {
    let mut release_callees: BTreeMap<u32, bool> = BTreeMap::new();
    let mut unguarded_callees = BTreeSet::new();
    let mut assign_candidates = BTreeSet::new();
    let mut alloc_candidates = BTreeSet::new();
    let mut mouse_getters = BTreeSet::new();

    let mut slots = BTreeMap::new();
    let mut rdx_rdata = false;
    let mut r8_imm = false;
    let mut rcx_imm = false;

    for (index, i) in builder.iter().enumerate() {
        if i.mnemonic() == Mnemonic::Call {
            let callee = call_target(image, i)
                .ok_or_else(|| invalid("indirect call inside cursor list"))?;
            let rcx = slots.get(&Register::RCX).copied();
            let any_stack_arg = [Register::RCX, Register::RDX, Register::R8, Register::R9]
                .into_iter()
                .any(|reg| slots.contains_key(&reg));
            if guarded.contains(&index) {
                let takes_obj = rcx.is_some();
                match release_callees.get(&callee) {
                    Some(&existing) if existing != takes_obj => {
                        return Err(invalid("release helper called with inconsistent signatures"));
                    }
                    None => {
                        release_callees.insert(callee, takes_obj);
                    }
                    _ => {}
                }
            } else {
                unguarded_callees.insert(callee);
                if callee != push_callee {
                    if index > last_push_in_builder && !any_stack_arg {
                        mouse_getters.insert(callee);
                    } else if rcx.is_some_and(|slot| slot != vec_slot) && rdx_rdata && r8_imm {
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

    if release_callees
        .keys()
        .any(|callee| unguarded_callees.contains(callee))
    {
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
    if mouse_getters.len() > 1 {
        return Err(invalid("several mouse getter calls precede cursor load"));
    }

    Ok(ClassifiedCalls {
        assign_callee: assign_candidates.into_iter().next(),
        alloc_callee: alloc_candidates.into_iter().next(),
        mouse_getter: mouse_getters.into_iter().next(),
        release_callees,
    })
}

fn verify_load_site(
    construction: &CursorConstruction<'_>,
    load_inst: &Instruction,
    loader: &LoaderInfo,
) -> io::Result<()> {
    if register_value(&construction.regs, Register::RDX)? != construction.vec_addr {
        return Err(invalid(
            "cursor loader does not receive the populated cursor vector",
        ));
    }
    let expected_va = construction.image.base + u64::from(loader.entry);
    if call_target(construction.image, load_inst) == Some(loader.entry) {
        return Ok(());
    }
    if load_inst.op0_kind() == OpKind::Register {
        let target = register_value(&construction.regs, load_inst.op0_register())?;
        if target != expected_va {
            return Err(invalid(
                "indirect cursor loader call does not resolve to the mouse loader slot",
            ));
        }
        return Ok(());
    }
    if load_inst.op0_kind() == OpKind::Memory {
        let slot_addr = memory_address(&construction.regs, load_inst)?;
        let target = construction.number_at(slot_addr, 8)?;
        if target != expected_va {
            return Err(invalid(
                "memory-indirect cursor loader call does not resolve to the mouse loader slot",
            ));
        }
        return Ok(());
    }
    Err(invalid("unsupported cursor loader call form"))
}

struct CursorConstruction<'a> {
    image: &'a PeImage,
    vec_addr: u64,
    push_callee: u32,
    assign_callee: Option<u32>,
    alloc_callee: Option<u32>,
    mouse_getter: Option<u32>,
    release_callees: BTreeSet<u32>,
    regs: Registers,
    vectors: BTreeMap<Register, Vec<u8>>,
    bytes: BTreeMap<u64, u8>,
    strings: BTreeMap<u64, String>,
    next_heap: u64,
    paths: Vec<String>,
}

impl<'a> CursorConstruction<'a> {
    #[allow(
        clippy::too_many_arguments,
        reason = "each argument is an independent verified role from the PE layout"
    )]
    fn new(
        image: &'a PeImage,
        frame_delta: Option<i64>,
        vec_addr: u64,
        push_callee: u32,
        assign_callee: Option<u32>,
        alloc_callee: Option<u32>,
        mouse_getter: Option<u32>,
        release_callees: BTreeSet<u32>,
        loader: &LoaderInfo,
    ) -> Self {
        let mut initial = vec![(Register::RSP, STACK)];
        if let Some(delta) = frame_delta {
            initial.push((Register::RBP, STACK.wrapping_add(delta as u64)));
        }
        let mut bytes = BTreeMap::new();
        for (offset, &byte) in MOUSE_VTABLE.to_le_bytes().iter().enumerate() {
            bytes.insert(MOUSE_OBJ + offset as u64, byte);
        }
        let loader_va = image.base + u64::from(loader.entry);
        for (offset, &byte) in loader_va.to_le_bytes().iter().enumerate() {
            bytes.insert(MOUSE_VTABLE + loader.vtable_offset + offset as u64, byte);
        }
        Self {
            image,
            vec_addr,
            push_callee,
            assign_callee,
            alloc_callee,
            mouse_getter,
            release_callees,
            regs: Registers::known(&initial),
            vectors: BTreeMap::new(),
            bytes,
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

    fn check_not_vector(&self, address: u64, length: usize) -> io::Result<()> {
        let end = address
            .checked_add(length as u64)
            .ok_or_else(|| invalid("cursor construction address overflow"))?;
        let vec_end = self.vec_addr.saturating_add(VECTOR_HEADER as u64);
        if address < vec_end && self.vec_addr < end {
            return Err(invalid("cursor vector header is mutated directly"));
        }
        Ok(())
    }

    fn store(&mut self, address: u64, bytes: &[u8]) -> io::Result<()> {
        if address >= self.image.base {
            return Err(invalid("cursor construction writes image storage"));
        }
        self.check_not_vector(address, bytes.len())?;
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
        self.check_not_vector(address, length)?;
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
        decode_msvc_string(|address, len| self.bytes_at(address, len), object)
    }

    fn source_bytes(&self, i: &Instruction) -> io::Result<Vec<u8>> {
        read_source_bytes(
            &self.regs,
            &self.vectors,
            |address, len| self.bytes_at(address, len),
            i,
        )
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
        let mut return_value = None;
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
            return_value = Some(address);
        } else if self.mouse_getter == Some(target) {
            return_value = Some(MOUSE_OBJ);
        } else if self.release_callees.contains(&target) {
        } else {
            return Err(invalid("unsupported call in cursor list"));
        }
        self.clobber_call();
        if let Some(value) = return_value {
            set_register(&mut self.regs, Register::RAX, value)?;
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

enum IatHook {
    None,
    Alloc { expected_size: u64, result: u64, called: bool },
    Free { expected_ptr: u64, called: bool },
}

struct MiniExec<'a> {
    image: &'a PeImage,
    regs: Registers,
    vectors: BTreeMap<Register, Vec<u8>>,
    bytes: BTreeMap<u64, u8>,
    written_outside_stack: BTreeSet<u64>,
    stack_low: u64,
    stack_high: u64,
    flags: (bool, bool, bool),
    alloc_override: Option<(u32, u64)>,
    iat: IatHook,
}

impl<'a> MiniExec<'a> {
    fn new(image: &'a PeImage, initial_regs: &[(Register, u64)]) -> Self {
        let stack_top = STACK + 0x1000;
        let mut regs = Registers::known(&[(Register::RSP, stack_top)]);
        for &(reg, value) in initial_regs {
            regs.set(reg, value);
        }
        let mut bytes = BTreeMap::new();
        for (offset, &byte) in SENTINEL_RET.to_le_bytes().iter().enumerate() {
            bytes.insert(stack_top + offset as u64, byte);
        }
        for (offset, &byte) in TLS_SLOT.to_le_bytes().iter().enumerate() {
            bytes.insert(TLS_ARRAY + offset as u64, byte);
        }
        for (offset, &byte) in i32::MAX.to_le_bytes().iter().enumerate() {
            bytes.insert(TLS_SLOT + 0x20ec + offset as u64, byte);
        }
        Self {
            image,
            regs,
            vectors: BTreeMap::new(),
            bytes,
            written_outside_stack: BTreeSet::new(),
            stack_low: STACK - 0x4000,
            stack_high: stack_top + 0x200,
            flags: (false, false, false),
            alloc_override: None,
            iat: IatHook::None,
        }
    }

    fn seed(&mut self, address: u64, data: &[u8]) {
        for (offset, &byte) in data.iter().enumerate() {
            self.bytes.insert(address + offset as u64, byte);
        }
    }

    fn read_bytes(&self, address: u64, length: usize) -> io::Result<Vec<u8>> {
        if address >= self.image.base {
            let rva = self.image.va_to_rva(address)?;
            if let Ok(slice) = self.image.bytes(rva, length) {
                return Ok(slice.to_vec());
            }
            if self.image.is_writable_virtual(rva, length) {
                return Ok(vec![0; length]);
            }
            return Err(invalid("helper read outside file-backed or writable image data"));
        }
        (0..length)
            .map(|offset| {
                self.bytes
                    .get(&address.wrapping_add(offset as u64))
                    .copied()
                    .ok_or_else(|| invalid("helper read unknown memory byte"))
            })
            .collect()
    }

    fn read_u64(&self, address: u64, length: usize) -> io::Result<u64> {
        let bytes = self.read_bytes(address, length)?;
        let mut buf = [0; 8];
        buf[..length].copy_from_slice(&bytes);
        Ok(u64::from_le_bytes(buf))
    }

    fn write_bytes(&mut self, address: u64, data: &[u8]) -> io::Result<()> {
        if address >= self.image.base {
            return Err(invalid("helper wrote into image memory"));
        }
        let in_stack =
            address >= self.stack_low && address.saturating_add(data.len() as u64) <= self.stack_high;
        for (offset, &byte) in data.iter().enumerate() {
            let at = address.wrapping_add(offset as u64);
            self.bytes.insert(at, byte);
            if !in_stack {
                self.written_outside_stack.insert(at);
            }
        }
        Ok(())
    }

    fn push_u64(&mut self, value: u64) -> io::Result<()> {
        let rsp = register_value(&self.regs, Register::RSP)?.wrapping_sub(8);
        set_register(&mut self.regs, Register::RSP, rsp)?;
        self.write_bytes(rsp, &value.to_le_bytes())
    }

    fn pop_u64(&mut self) -> io::Result<u64> {
        let rsp = register_value(&self.regs, Register::RSP)?;
        let value = self.read_u64(rsp, 8)?;
        set_register(&mut self.regs, Register::RSP, rsp.wrapping_add(8))?;
        Ok(value)
    }

    fn run(&mut self, entry: u32) -> io::Result<()> {
        let mut ip = self.image.base + u64::from(entry);
        for _ in 0..MAX_HELPER_STEPS {
            if ip == SENTINEL_RET {
                return Ok(());
            }
            let rva = self.image.va_to_rva(ip)?;
            let slice = self.image.bytes(rva, 15.min(remaining_executable(self.image, rva)?))?;
            let i = Decoder::with_ip(64, slice, ip, DecoderOptions::NONE).decode();
            if i.is_invalid() {
                return Err(invalid("helper contains invalid instruction"));
            }
            let mut next = i.next_ip();
            match i.mnemonic() {
                Mnemonic::Push => {
                    let value = match i.op0_kind() {
                        OpKind::Register => self.regs.value(i.op0_register()).unwrap_or(0),
                        _ => immediate(&i, 0)?,
                    };
                    self.push_u64(value)?;
                }
                Mnemonic::Pop => {
                    let value = self.pop_u64()?;
                    set_register(&mut self.regs, i.op0_register(), value)?;
                }
                Mnemonic::Mov
                    if i.segment_prefix() == Register::GS
                        && i.op0_kind() == OpKind::Register
                        && i.op1_kind() == OpKind::Memory
                        && i.memory_displacement64() == 0x58 =>
                {
                    set_register(&mut self.regs, i.op0_register(), TLS_ARRAY)?;
                }
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
                    let value = read_source_bytes(
                        &self.regs,
                        &self.vectors,
                        |address, len| self.read_bytes(address, len),
                        &i,
                    );
                    match i.op0_kind() {
                        OpKind::Register if i.op0_register().size() > 8 => {
                            let bytes = value?;
                            self.vectors.insert(i.op0_register(), bytes);
                        }
                        OpKind::Register => {
                            let bytes = value?;
                            let mut scalar = [0; 8];
                            scalar[..bytes.len()].copy_from_slice(&bytes);
                            set_register(
                                &mut self.regs,
                                i.op0_register(),
                                u64::from_le_bytes(scalar),
                            )?;
                        }
                        OpKind::Memory => {
                            let address = memory_address(&self.regs, &i)?;
                            let bytes = match value {
                                Ok(bytes) => bytes,
                                Err(_)
                                    if address >= self.stack_low
                                        && address < self.stack_high =>
                                {
                                    vec![0; i.memory_size().size()]
                                }
                                Err(error) => return Err(error),
                            };
                            self.write_bytes(address, &bytes)?;
                        }
                        _ => return Err(invalid("helper unsupported move destination")),
                    }
                }
                Mnemonic::Lea => {
                    let address = memory_address(&self.regs, &i)?;
                    set_register(&mut self.regs, i.op0_register(), address)?;
                }
                Mnemonic::Cdqe => {
                    let value = register_value(&self.regs, Register::EAX)?;
                    set_register(&mut self.regs, Register::RAX, signed(value, 4) as u64)?;
                }
                Mnemonic::Xor
                    if i.op0_kind() == OpKind::Register
                        && i.op1_kind() == OpKind::Register
                        && i.op0_register() == i.op1_register() =>
                {
                    set_register(&mut self.regs, i.op0_register(), 0)?;
                    self.flags = (true, false, false);
                }
                Mnemonic::Xorps | Mnemonic::Xorpd | Mnemonic::Pxor
                    if i.op0_kind() == OpKind::Register
                        && i.op1_kind() == OpKind::Register
                        && i.op0_register() == i.op1_register() =>
                {
                    self.vectors.insert(i.op0_register(), vec![0; 16]);
                }
                Mnemonic::Add | Mnemonic::Sub | Mnemonic::Or | Mnemonic::And | Mnemonic::Xor => {
                    let width = operand_width(&i, 0)?;
                    let a = self.read_operand(&i, 0, width)?;
                    let b = self.read_operand(&i, 1, width)?;
                    let result = match i.mnemonic() {
                        Mnemonic::Add => a.wrapping_add(b),
                        Mnemonic::Sub => a.wrapping_sub(b),
                        Mnemonic::Or => a | b,
                        Mnemonic::And => a & b,
                        _ => a ^ b,
                    } & mask(width);
                    self.flags = if i.mnemonic() == Mnemonic::Sub {
                        compare(a, b, width)
                    } else {
                        compare(result, 0, width)
                    };
                    self.write_operand(&i, 0, width, result)?;
                }
                Mnemonic::Inc | Mnemonic::Dec => {
                    let width = operand_width(&i, 0)?;
                    let a = self.read_operand(&i, 0, width)?;
                    let result = if i.mnemonic() == Mnemonic::Inc {
                        a.wrapping_add(1)
                    } else {
                        a.wrapping_sub(1)
                    } & mask(width);
                    self.flags = compare(result, 0, width);
                    self.write_operand(&i, 0, width, result)?;
                }
                Mnemonic::Shl | Mnemonic::Shr => {
                    let width = operand_width(&i, 0)?;
                    let a = self.read_operand(&i, 0, width)?;
                    let shift = (self.read_operand(&i, 1, 1)? & 63) as u32;
                    let result = if i.mnemonic() == Mnemonic::Shl {
                        a.wrapping_shl(shift)
                    } else {
                        a.wrapping_shr(shift)
                    } & mask(width);
                    self.flags = compare(result, 0, width);
                    self.write_operand(&i, 0, width, result)?;
                }
                Mnemonic::Cmp => {
                    let width = operand_width(&i, 0)?;
                    let a = self.read_operand(&i, 0, width)?;
                    let b = self.read_operand(&i, 1, width)?;
                    self.flags = compare(a, b, width);
                }
                Mnemonic::Test => {
                    let width = operand_width(&i, 0)?;
                    let a = self.read_operand(&i, 0, width)?;
                    let b = self.read_operand(&i, 1, width)?;
                    self.flags = compare(a & b, 0, width);
                }
                Mnemonic::Cmove | Mnemonic::Cmovne | Mnemonic::Cmovb | Mnemonic::Cmovbe | Mnemonic::Cmova | Mnemonic::Cmovae | Mnemonic::Cmovg | Mnemonic::Cmovl => {
                    let (equal, below, less) = self.flags;
                    let take = match i.mnemonic() {
                        Mnemonic::Cmove => equal,
                        Mnemonic::Cmovne => !equal,
                        Mnemonic::Cmovb => below,
                        Mnemonic::Cmovbe => equal || below,
                        Mnemonic::Cmova => !equal && !below,
                        Mnemonic::Cmovae => !below,
                        Mnemonic::Cmovg => !equal && !less,
                        _ => less,
                    };
                    if take {
                        let width = operand_width(&i, 0)?;
                        let value = self.read_operand(&i, 1, width)?;
                        set_register(&mut self.regs, i.op0_register(), value)?;
                    }
                }
                Mnemonic::Jmp => {
                    next = match i.op0_kind() {
                        OpKind::NearBranch16 | OpKind::NearBranch32 | OpKind::NearBranch64 => {
                            i.near_branch_target()
                        }
                        OpKind::Register => register_value(&self.regs, i.op0_register())?,
                        _ => return Err(invalid("helper unsupported indirect jump")),
                    };
                }
                Mnemonic::Je
                | Mnemonic::Jne
                | Mnemonic::Jg
                | Mnemonic::Jge
                | Mnemonic::Jl
                | Mnemonic::Jle
                | Mnemonic::Ja
                | Mnemonic::Jae
                | Mnemonic::Jb
                | Mnemonic::Jbe => {
                    let (equal, below, less) = self.flags;
                    let taken = match i.mnemonic() {
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
                        next = i.near_branch_target();
                    }
                }
                Mnemonic::Call => {
                    if let Some(target) = call_target(self.image, &i) {
                        if let Some((alloc_rva, result)) = self.alloc_override
                            && target == alloc_rva
                        {
                            set_register(&mut self.regs, Register::RAX, result)?;
                        } else {
                            self.push_u64(next)?;
                            next = self.image.base + u64::from(target);
                        }
                    } else if i.op0_kind() == OpKind::Memory && i.is_ip_rel_memory_operand() {
                        let rva = self.image.va_to_rva(i.ip_rel_memory_address())?;
                        if !self.image.is_read_only(rva, 8) {
                            return Err(invalid("helper indirect call does not target read-only IAT"));
                        }
                        match &mut self.iat {
                            IatHook::Alloc {
                                expected_size,
                                result,
                                called,
                            } => {
                                let size_ok = self.regs.value(Register::R8) == Some(*expected_size)
                                    || self.regs.value(Register::RCX) == Some(*expected_size);
                                if !size_ok {
                                    return Err(invalid(
                                        "allocator IAT call did not pass the requested byte count",
                                    ));
                                }
                                *called = true;
                                set_register(&mut self.regs, Register::RAX, *result)?;
                            }
                            IatHook::Free {
                                expected_ptr,
                                called,
                            } => {
                                let ptr_ok = self.regs.value(Register::R8) == Some(*expected_ptr)
                                    || self.regs.value(Register::RCX) == Some(*expected_ptr);
                                if !ptr_ok {
                                    return Err(invalid(
                                        "release IAT call did not pass the allocated pointer",
                                    ));
                                }
                                *called = true;
                                set_register(&mut self.regs, Register::RAX, 1)?;
                            }
                            IatHook::None => {
                                return Err(invalid("unexpected indirect call in verified helper"));
                            }
                        }
                    } else {
                        return Err(invalid("unexpected indirect call in verified helper"));
                    }
                }
                Mnemonic::Ret => {
                    next = self.pop_u64()?;
                }
                Mnemonic::Nop => {}
                _ => {
                    return Err(invalid(format!(
                        "unsupported instruction {:?} in verified helper",
                        i.mnemonic()
                    )));
                }
            }
            ip = next;
        }
        Err(invalid("verified helper exceeded instruction step budget"))
    }

    fn read_operand(&self, i: &Instruction, operand: u32, width: usize) -> io::Result<u64> {
        match i.op_kind(operand) {
            OpKind::Register => register_value(&self.regs, i.op_register(operand)),
            OpKind::Memory => self.read_u64(memory_address(&self.regs, i)?, width),
            _ => immediate(i, operand),
        }
    }

    fn write_operand(&mut self, i: &Instruction, operand: u32, width: usize, value: u64) -> io::Result<()> {
        match i.op_kind(operand) {
            OpKind::Register => set_register(&mut self.regs, i.op_register(operand), value),
            OpKind::Memory => {
                let address = memory_address(&self.regs, i)?;
                self.write_bytes(address, &value.to_le_bytes()[..width])
            }
            _ => Err(invalid("helper unsupported write operand")),
        }
    }
}

fn remaining_executable(image: &PeImage, rva: u32) -> io::Result<usize> {
    for &(start, end) in image.executable_ranges() {
        if start <= rva && rva < end {
            return Ok((end - rva) as usize);
        }
    }
    Err(invalid("helper execution left executable code"))
}

fn verify_init(image: &PeImage, init_callee: u32) -> io::Result<()> {
    const VEC_OBJ: u64 = 0x20_0000;
    let mut exec = MiniExec::new(image, &[(Register::RCX, VEC_OBJ)]);
    exec.seed(VEC_OBJ, &[0xaa; VECTOR_HEADER]);
    exec.run(init_callee)?;
    if exec.read_u64(VEC_OBJ, 8)? != 0 || exec.read_u64(VEC_OBJ + 8, 8)? != 0 {
        return Err(invalid("cursor vector initializer did not zero header fields"));
    }
    if exec
        .written_outside_stack
        .iter()
        .any(|&at| !(VEC_OBJ..VEC_OBJ + VECTOR_HEADER as u64).contains(&at))
    {
        return Err(invalid("cursor vector initializer wrote outside the vector"));
    }
    Ok(())
}

fn verify_push(image: &PeImage, push_callee: u32) -> io::Result<()> {
    const VEC_OBJ: u64 = 0x20_0000;
    const VEC_BUF: u64 = 0x21_0000;
    const SRC_STR: u64 = 0x22_0000;
    const SRC_HEAP: u64 = 0x23_0000;

    let run_case = |src_bytes: &[u8; STRING_OBJECT], expected: &str| -> io::Result<()> {
        let mut exec = MiniExec::new(image, &[(Register::RCX, VEC_OBJ), (Register::RDX, SRC_STR)]);
        let mut header = [0u8; VECTOR_HEADER];
        header[0..8].copy_from_slice(&VEC_BUF.to_le_bytes());
        header[8..12].copy_from_slice(&4_u32.to_le_bytes());
        header[12..16].copy_from_slice(&1_u32.to_le_bytes());
        exec.seed(VEC_OBJ, &header);
        exec.seed(VEC_BUF, &[0u8; 4 * STRING_OBJECT]);
        exec.seed(SRC_STR, src_bytes);
        exec.seed(SRC_HEAP, b"0123456789abcdefghij\0");
        exec.run(push_callee)?;

        if exec.read_u64(VEC_OBJ, 8)? != VEC_BUF
            || exec.read_u64(VEC_OBJ + 8, 4)? != 4
            || exec.read_u64(VEC_OBJ + 12, 4)? != 2
        {
            return Err(invalid("cursor push helper did not append to vector count"));
        }
        let pushed = decode_msvc_string(|a, l| exec.read_bytes(a, l), VEC_BUF + STRING_OBJECT as u64)?;
        if pushed != expected {
            return Err(invalid("cursor push helper did not store the pushed string"));
        }
        let allowed = |at: u64| {
            (VEC_OBJ..VEC_OBJ + VECTOR_HEADER as u64).contains(&at)
                || (VEC_BUF + STRING_OBJECT as u64..VEC_BUF + 2 * STRING_OBJECT as u64).contains(&at)
                || (SRC_STR..SRC_STR + STRING_OBJECT as u64).contains(&at)
        };
        if exec.written_outside_stack.iter().any(|&at| !allowed(at)) {
            return Err(invalid("cursor push helper wrote unexpected memory"));
        }
        Ok(())
    };

    let mut sso = [0u8; STRING_OBJECT];
    sso[0..6].copy_from_slice(b"hello\0");
    sso[16..24].copy_from_slice(&5_u64.to_le_bytes());
    sso[24..32].copy_from_slice(&15_u64.to_le_bytes());
    run_case(&sso, "hello")?;

    let mut heap = [0u8; STRING_OBJECT];
    heap[0..8].copy_from_slice(&SRC_HEAP.to_le_bytes());
    heap[16..24].copy_from_slice(&20_u64.to_le_bytes());
    heap[24..32].copy_from_slice(&31_u64.to_le_bytes());
    run_case(&heap, "0123456789abcdefghij")?;
    Ok(())
}

fn verify_alloc(image: &PeImage, alloc_callee: u32) -> io::Result<()> {
    const TEST_ALLOC_PTR: u64 = 0x40_0000;
    let mut exec = MiniExec::new(image, &[(Register::RCX, 0x20)]);
    exec.iat = IatHook::Alloc {
        expected_size: 0x20,
        result: TEST_ALLOC_PTR,
        called: false,
    };
    exec.run(alloc_callee)?;
    let called = matches!(exec.iat, IatHook::Alloc { called: true, .. });
    if !called || register_value(&exec.regs, Register::RAX)? != TEST_ALLOC_PTR {
        return Err(invalid("cursor allocator helper did not return an allocated buffer"));
    }
    if !exec.written_outside_stack.is_empty() {
        return Err(invalid("cursor allocator helper wrote caller memory"));
    }
    Ok(())
}

fn verify_assign(
    image: &PeImage,
    assign_callee: u32,
    alloc_callee: Option<u32>,
) -> io::Result<()> {
    const OBJ: u64 = 0x20_0000;
    const SRC: u64 = 0x21_0000;
    const TEST_HEAP: u64 = 0x40_0000;

    let mut sso = MiniExec::new(
        image,
        &[(Register::RCX, OBJ), (Register::RDX, SRC), (Register::R8, 5)],
    );
    sso.seed(OBJ, &[0u8; STRING_OBJECT]);
    sso.seed(SRC, b"abcde\0");
    sso.run(assign_callee)?;
    let decoded = decode_msvc_string(|a, l| sso.read_bytes(a, l), OBJ)?;
    if decoded != "abcde" {
        return Err(invalid("cursor string assign helper failed SSO verification"));
    }
    if sso
        .written_outside_stack
        .iter()
        .any(|&at| !(OBJ..OBJ + STRING_OBJECT as u64).contains(&at))
    {
        return Err(invalid("cursor string assign helper wrote outside target string"));
    }

    let long_text = b"0123456789abcdefghij";
    let mut heap = MiniExec::new(
        image,
        &[
            (Register::RCX, OBJ),
            (Register::RDX, SRC),
            (Register::R8, long_text.len() as u64),
        ],
    );
    heap.seed(OBJ, &[0u8; STRING_OBJECT]);
    heap.seed(SRC, long_text);
    heap.seed(SRC + long_text.len() as u64, &[0]);
    if let Some(alloc) = alloc_callee {
        heap.alloc_override = Some((alloc, TEST_HEAP));
    } else {
        heap.iat = IatHook::Alloc {
            expected_size: 32,
            result: TEST_HEAP,
            called: false,
        };
    }
    heap.run(assign_callee)?;
    let decoded = decode_msvc_string(|a, l| heap.read_bytes(a, l), OBJ)?;
    if decoded != "0123456789abcdefghij" {
        return Err(invalid("cursor string assign helper failed heap verification"));
    }
    let allowed = |at: u64| {
        (OBJ..OBJ + STRING_OBJECT as u64).contains(&at)
            || (TEST_HEAP..TEST_HEAP + 64).contains(&at)
    };
    if heap.written_outside_stack.iter().any(|&at| !allowed(at)) {
        return Err(invalid("cursor string assign helper wrote unexpected memory"));
    }
    Ok(())
}

fn verify_release(image: &PeImage, release_callee: u32, takes_string_obj: bool) -> io::Result<()> {
    const OBJ: u64 = 0x20_0000;
    const HEAP_PTR: u64 = 0x40_0000;
    let initial: Vec<(Register, u64)> = if takes_string_obj {
        vec![
            (Register::RCX, OBJ),
            (Register::RDX, HEAP_PTR),
            (Register::R8, 31),
        ]
    } else {
        vec![(Register::RCX, HEAP_PTR), (Register::RDX, 32)]
    };
    let mut exec = MiniExec::new(image, &initial);
    exec.seed(OBJ, &[0xbb; STRING_OBJECT]);
    exec.iat = IatHook::Free {
        expected_ptr: HEAP_PTR,
        called: false,
    };
    exec.run(release_callee)?;
    let called = matches!(exec.iat, IatHook::Free { called: true, .. });
    if !called {
        return Err(invalid("cursor release helper did not reach deallocation"));
    }
    if !exec.written_outside_stack.is_empty() {
        return Err(invalid("cursor release helper mutated caller memory"));
    }
    Ok(())
}

fn decode_msvc_string(
    read: impl Fn(u64, usize) -> io::Result<Vec<u8>>,
    object: u64,
) -> io::Result<String> {
    let number = |address: u64| -> io::Result<u64> {
        let bytes = read(address, 8)?;
        Ok(u64::from_le_bytes(bytes.try_into().unwrap()))
    };
    let length = usize::try_from(number(object + 16)?)
        .map_err(|_| invalid("cursor string length overflow"))?;
    let capacity = number(object + 24)?;
    if length as u64 > capacity {
        return Err(invalid("cursor string length exceeds capacity"));
    }
    let payload = if capacity <= 15 {
        object
    } else {
        number(object)?
    };
    let bytes = read(
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

fn read_source_bytes(
    regs: &Registers,
    vectors: &BTreeMap<Register, Vec<u8>>,
    read: impl Fn(u64, usize) -> io::Result<Vec<u8>>,
    i: &Instruction,
) -> io::Result<Vec<u8>> {
    let width = if i.op0_kind() == OpKind::Memory {
        i.memory_size().size()
    } else {
        i.op0_register().size()
    };
    if width == 0 || width > 16 {
        return Err(invalid("unsupported cursor construction width"));
    }
    let source_width = match i.op1_kind() {
        OpKind::Memory => i.memory_size().size(),
        OpKind::Register => i.op1_register().size(),
        _ => width,
    };
    let mut bytes = match i.op1_kind() {
        OpKind::Memory => read(memory_address(regs, i)?, source_width)?,
        OpKind::Register if i.op1_register().size() > 8 => vectors
            .get(&i.op1_register())
            .cloned()
            .ok_or_else(|| invalid("cursor construction vector is unknown"))?,
        OpKind::Register => register_value(regs, i.op1_register())?
            .to_le_bytes()
            .to_vec(),
        _ => immediate(i, 1)?.to_le_bytes().to_vec(),
    };
    if matches!(i.mnemonic(), Mnemonic::Movsx | Mnemonic::Movsxd) {
        if source_width == 0 || source_width > 8 || width > 8 {
            return Err(invalid("unsupported sign-extension width"));
        }
        let mut scalar = [0; 8];
        scalar[..source_width].copy_from_slice(&bytes[..source_width]);
        let extended = signed(u64::from_le_bytes(scalar), source_width) as u64;
        bytes = extended.to_le_bytes().to_vec();
    }
    bytes.resize(width, 0);
    bytes.truncate(width);
    Ok(bytes)
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

fn operand_width(i: &Instruction, operand: u32) -> io::Result<usize> {
    let width = match i.op_kind(operand) {
        OpKind::Register => i.op_register(operand).size(),
        OpKind::Memory => i.memory_size().size(),
        _ => 8,
    };
    if width == 0 || width > 8 {
        return Err(invalid("unsupported operand width"));
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

    const SYNTH_LOG: &str = "synth_cursor_log";
    const SYNTH_ERR: &str = "synth_cursor_err";
    const SYNTH_MOUSE_RTTI: &str = ".?AVSynthMouse@@";

    const MAIN_FN: u32 = TEXT;
    const INIT_FN: u32 = TEXT + 0x500;
    const ASSIGN_FN: u32 = TEXT + 0x540;
    const PUSH_FN: u32 = TEXT + 0x620;
    const ALLOC_FN: u32 = TEXT + 0x700;
    const FREE_FN: u32 = TEXT + 0x740;
    const MOUSE_FN: u32 = TEXT + 0x780;
    const LOADER_FN: u32 = TEXT + 0x7c0;
    const DUMMY_FN: u32 = TEXT + 0x820;

    const IAT_ALLOC: u32 = RDATA + 0x500;
    const IAT_FREE: u32 = RDATA + 0x508;
    const RTTI_TYPE: u32 = RDATA + 0x540;
    const RTTI_LOCATOR: u32 = RDATA + 0x580;
    const MOUSE_VTABLE_RVA: u32 = RDATA + 0x600;
    const LOADER_SLOT: u8 = 0x18;

    fn synthetic_anchors() -> Anchors {
        let hashed = |text: &str| Hashed {
            length: text.len(),
            fnv: fnv1a(text.as_bytes()),
        };
        Anchors {
            loading_log: Anchor {
                what: "synthetic cursor log",
                text: hashed(SYNTH_LOG),
            },
            load_error: Anchor {
                what: "synthetic cursor error",
                text: hashed(SYNTH_ERR),
            },
            mouse_type: Anchor {
                what: "synthetic mouse type",
                text: hashed(SYNTH_MOUSE_RTTI),
            },
        }
    }

    fn emit_synth_init() -> Code {
        Code::at(INIT_FN)
            .raw(&[0x48, 0xc7, 0x01, 0, 0, 0, 0])
            .raw(&[0x48, 0xc7, 0x41, 0x08, 0, 0, 0, 0])
            .raw(&[0x48, 0x89, 0xc8])
            .ret()
    }

    fn emit_synth_alloc() -> Code {
        Code::at(ALLOC_FN)
            .raw(&[0x49, 0x89, 0xc8])
            .raw(&[0x31, 0xd2])
            .raw(&[0x31, 0xc9])
            .rip(&[0xff, 0x15], IAT_ALLOC)
            .ret()
    }

    fn emit_synth_free(mutate_caller: bool) -> Code {
        let mut code = Code::at(FREE_FN);
        if mutate_caller {
            code = code.raw(&[0xc6, 0x01, 0x00]);
        }
        code.raw(&[0x49, 0x89, 0xd0])
            .raw(&[0x31, 0xd2])
            .raw(&[0x31, 0xc9])
            .rip(&[0xff, 0x15], IAT_FREE)
            .ret()
    }

    fn emit_synth_push(inert: bool) -> Code {
        if inert {
            return Code::at(PUSH_FN).ret();
        }
        Code::at(PUSH_FN)
            .raw(&[0x48, 0x63, 0x41, 0x0c])
            .raw(&[0x44, 0x8b, 0x41, 0x08])
            .raw(&[0x44, 0x39, 0xc0])
            .raw(&[0x74, 0x33])
            .raw(&[0x49, 0x89, 0xc1])
            .raw(&[0x49, 0xc1, 0xe1, 0x05])
            .raw(&[0x4c, 0x03, 0x09])
            .raw(&[0x0f, 0x10, 0x02])
            .raw(&[0x41, 0x0f, 0x11, 0x01])
            .raw(&[0x0f, 0x10, 0x4a, 0x10])
            .raw(&[0x41, 0x0f, 0x11, 0x49, 0x10])
            .raw(&[0x48, 0xc7, 0x42, 0x10, 0, 0, 0, 0])
            .raw(&[0x48, 0xc7, 0x42, 0x18, 0x0f, 0, 0, 0])
            .raw(&[0xc6, 0x02, 0x00])
            .raw(&[0xff, 0x41, 0x0c])
            .raw(&[0x4c, 0x89, 0xc8])
            .ret()
    }

    fn emit_synth_assign() -> Code {
        Code::at(ASSIGN_FN)
            .raw(&[0x53])
            .raw(&[0x56])
            .raw(&[0x57])
            .raw(&[0x48, 0x89, 0xcb])
            .raw(&[0x48, 0x89, 0xd6])
            .raw(&[0x4c, 0x89, 0xc7])
            .raw(&[0x49, 0x83, 0xf8, 0x0f])
            .raw(&[0x77, 0x11])
            .raw(&[0x48, 0x89, 0x7b, 0x10])
            .raw(&[0x48, 0xc7, 0x43, 0x18, 0x0f, 0, 0, 0])
            .raw(&[0x48, 0x89, 0xd8])
            .raw(&[0xeb, 0x19])
            .raw(&[0xb9, 0x20, 0x00, 0x00, 0x00])
            .call(ALLOC_FN)
            .raw(&[0x48, 0x89, 0x03])
            .raw(&[0x48, 0x89, 0x7b, 0x10])
            .raw(&[0x48, 0xc7, 0x43, 0x18, 0x1f, 0, 0, 0])
            .raw(&[0x45, 0x31, 0xc9])
            .raw(&[0x49, 0x39, 0xf9])
            .raw(&[0x74, 0x0e])
            .raw(&[0x42, 0x0f, 0xb6, 0x14, 0x0e])
            .raw(&[0x42, 0x88, 0x14, 0x08])
            .raw(&[0x49, 0xff, 0xc1])
            .raw(&[0xeb, 0xed])
            .raw(&[0x42, 0xc6, 0x04, 0x08, 0x00])
            .raw(&[0x5f])
            .raw(&[0x5e])
            .raw(&[0x5b])
            .ret()
    }

    fn emit_synth_loader(err_rva: u32, capacity: u8) -> Code {
        Code::at(LOADER_FN)
            .rip(&[0x48, 0x8d, 0x15], err_rva)
            .raw(&[0x31, 0xdb])
            .raw(&[0xff, 0xc3])
            .raw(&[0x83, 0xfb, capacity])
            .raw(&[0x7c, 0xf9])
            .ret()
    }

    struct FixtureOptions {
        prologue: bool,
        use_rbp_for_string: bool,
        inert_push: bool,
        mutating_free: bool,
        intervening_vec_call: bool,
        wrong_loader_slot: bool,
    }

    impl Default for FixtureOptions {
        fn default() -> Self {
            Self {
                prologue: true,
                use_rbp_for_string: true,
                inert_push: false,
                mutating_free: false,
                intervening_vec_call: false,
                wrong_loader_slot: false,
            }
        }
    }

    fn build_synthetic_image(opts: FixtureOptions) -> PeImage {
        let mut image = SyntheticImage::new();
        let log_rva = RDATA;
        let err_rva = RDATA + 0x40;
        let first_rva = RDATA + 0x80;
        let empty_rva = RDATA + 0xc0;
        let inline_rva = RDATA + 0x100;

        let first = "gfx/cursors/alpha.cur";
        let inline_str = "gfx/alt/inline_01.ani";
        assert_eq!(inline_str.len(), 21);

        image.text(log_rva, SYNTH_LOG);
        image.text(err_rva, SYNTH_ERR);
        image.text(first_rva, first);
        image.text(empty_rva, "");
        image.text(inline_rva, inline_str);

        image.pointer(IAT_ALLOC, DUMMY_FN);
        image.pointer(IAT_FREE, DUMMY_FN);

        image.text(RTTI_TYPE + 16, SYNTH_MOUSE_RTTI);
        let mut locator = [0u8; 24];
        locator[0..4].copy_from_slice(&1_u32.to_le_bytes());
        locator[4..8].copy_from_slice(&0_u32.to_le_bytes());
        locator[12..16].copy_from_slice(&RTTI_TYPE.to_le_bytes());
        locator[20..24].copy_from_slice(&RTTI_LOCATOR.to_le_bytes());
        image.put(RTTI_LOCATOR, &locator);

        image.pointer(MOUSE_VTABLE_RVA - 8, RTTI_LOCATOR);
        for slot in 0..5u32 {
            let target = if slot == u32::from(LOADER_SLOT) / 8 {
                LOADER_FN
            } else {
                DUMMY_FN
            };
            image.pointer(MOUSE_VTABLE_RVA + slot * 8, target);
        }

        image.function(emit_synth_init());
        image.function(emit_synth_alloc());
        image.function(emit_synth_free(opts.mutating_free));
        image.function(emit_synth_push(opts.inert_push));
        image.function(emit_synth_assign());
        image.function(emit_synth_loader(err_rva, 4));
        image.function(Code::at(MOUSE_FN).ret());
        image.function(Code::at(DUMMY_FN).ret());

        let mut main = Code::at(MAIN_FN);
        if opts.prologue {
            main = main
                .raw(&[0x55])
                .raw(&[0x48, 0x81, 0xec, 0x00, 0x01, 0x00, 0x00])
                .raw(&[0x48, 0x8d, 0x6c, 0x24, 0x38]);
        }
        main = main
            .rip(&[0x48, 0x8d, 0x15], log_rva)
            .raw(&[0x48, 0x8d, 0x4c, 0x24, 0x60])
            .call(INIT_FN);

        main = main
            .raw(&[0x41, 0xb8])
            .raw(&(first.len() as u32).to_le_bytes())
            .rip(&[0x48, 0x8d, 0x15], first_rva)
            .raw(if opts.use_rbp_for_string {
                &[0x48, 0x8d, 0x4d, 0x08][..]
            } else {
                &[0x48, 0x8d, 0x4c, 0x24, 0x40][..]
            })
            .call(ASSIGN_FN)
            .raw(&[0x48, 0x8d, 0x54, 0x24, 0x40])
            .raw(&[0x48, 0x8d, 0x4c, 0x24, 0x60])
            .call(PUSH_FN)
            .raw(&[0x4c, 0x8b, 0x44, 0x24, 0x58])
            .raw(&[0x49, 0x83, 0xf8, 0x0f])
            .raw(&[0x76, 0x0f])
            .raw(&[0x48, 0x8b, 0x54, 0x24, 0x40])
            .raw(&[0x48, 0x8d, 0x4c, 0x24, 0x40])
            .call(FREE_FN);

        main = main
            .raw(&[0x41, 0xb8, 0, 0, 0, 0])
            .rip(&[0x48, 0x8d, 0x15], empty_rva)
            .raw(&[0x48, 0x8d, 0x4c, 0x24, 0x40])
            .call(ASSIGN_FN)
            .raw(&[0x48, 0x8d, 0x54, 0x24, 0x40])
            .raw(&[0x48, 0x8d, 0x4c, 0x24, 0x60])
            .call(PUSH_FN);

        main = main
            .raw(&[0xb9, 0x20, 0, 0, 0])
            .call(ALLOC_FN)
            .raw(&[0x48, 0x89, 0x44, 0x24, 0x40])
            .raw(&[0x48, 0xc7, 0x44, 0x24, 0x50, 21, 0, 0, 0])
            .raw(&[0x48, 0xc7, 0x44, 0x24, 0x58, 31, 0, 0, 0])
            .rip(&[0x4c, 0x8b, 0x05], inline_rva)
            .raw(&[0x4c, 0x89, 0x00])
            .rip(&[0x4c, 0x8b, 0x05], inline_rva + 8)
            .raw(&[0x4c, 0x89, 0x40, 0x08])
            .rip(&[0x44, 0x8b, 0x05], inline_rva + 16)
            .raw(&[0x44, 0x89, 0x40, 0x10])
            .rip(&[0x44, 0x0f, 0xb6, 0x05], inline_rva + 20)
            .raw(&[0x44, 0x88, 0x40, 0x14])
            .raw(&[0xc6, 0x40, 0x15, 0x00])
            .raw(&[0x48, 0x8d, 0x54, 0x24, 0x40])
            .raw(&[0x48, 0x8d, 0x4c, 0x24, 0x60])
            .call(PUSH_FN);

        if opts.intervening_vec_call {
            main = main
                .raw(&[0x48, 0x8d, 0x4c, 0x24, 0x60])
                .call(DUMMY_FN);
        }

        let dispatch_slot = if opts.wrong_loader_slot {
            LOADER_SLOT + 8
        } else {
            LOADER_SLOT
        };
        main = main
            .call(MOUSE_FN)
            .raw(&[0x48, 0x8b, 0x08])
            .raw(&[0x4c, 0x8b, 0x41, dispatch_slot])
            .raw(&[0x48, 0x8d, 0x54, 0x24, 0x60])
            .raw(&[0x48, 0x8b, 0xc8])
            .raw(&[0x41, 0xff, 0xd0])
            .ret();

        image.function(main);
        image.image()
    }

    #[test]
    fn decodes_verified_helpers_prologue_frame_and_isolates_empty_slots() {
        let image = build_synthetic_image(FixtureOptions::default());
        let table = CursorTable::decode_with_anchors(
            &image,
            synthetic_anchors(),
            &[
                (CursorAction::Normal, 0),
                (CursorAction::NoMove, 1),
                (CursorAction::DragSelect, 2),
                (CursorAction::AttackMove, 3),
            ],
        )
        .unwrap();
        assert_eq!(table.slots.len(), 4);
        assert_eq!(table.path_for(CursorAction::Normal), Ok("gfx/cursors/alpha.cur"));
        assert!(
            table
                .path_for(CursorAction::NoMove)
                .unwrap_err()
                .contains("empty cursor path")
        );
        assert_eq!(
            table.path_for(CursorAction::DragSelect),
            Ok("gfx/alt/inline_01.ani")
        );
        assert!(
            table
                .path_for(CursorAction::AttackMove)
                .unwrap_err()
                .contains("not populated")
        );
        assert!(
            table
                .path_for(CursorAction::PingOffensive)
                .unwrap_err()
                .contains("not verified for this build")
        );
    }

    #[test]
    fn refuses_inert_push_mutating_free_intervening_vector_call_wrong_slot_and_unproven_rbp() {
        for (opts, expected) in [
            (
                FixtureOptions {
                    inert_push: true,
                    ..FixtureOptions::default()
                },
                "cursor push helper",
            ),
            (
                FixtureOptions {
                    mutating_free: true,
                    ..FixtureOptions::default()
                },
                "mutated caller memory",
            ),
            (
                FixtureOptions {
                    intervening_vec_call: true,
                    ..FixtureOptions::default()
                },
                "unverified call touches cursor vector",
            ),
            (
                FixtureOptions {
                    wrong_loader_slot: true,
                    ..FixtureOptions::default()
                },
                "verified cursor loader call",
            ),
            (
                FixtureOptions {
                    prologue: false,
                    use_rbp_for_string: true,
                    ..FixtureOptions::default()
                },
                "register value is unknown",
            ),
        ] {
            let image = build_synthetic_image(opts);
            let error = CursorTable::decode_with_anchors(&image, synthetic_anchors(), &[])
                .unwrap_err()
                .to_string();
            assert!(error.contains(expected), "expected {expected:?} in {error:?}");
        }
    }

    #[test]
    fn movsx_sign_extends_from_the_source_register_width() {
        let mut regs = Registers::known(&[(Register::RAX, 0x1234_56ff)]);
        let vectors = BTreeMap::new();
        let mut decoder = Decoder::with_ip(
            64,
            &[0x0f, 0xbe, 0xc0, 0x48, 0x0f, 0xbe, 0xc0],
            0x1000,
            DecoderOptions::NONE,
        );
        let movsx_eax_al = decoder.decode();
        let movsx_rax_al = decoder.decode();
        let bytes32 =
            read_source_bytes(&regs, &vectors, |_, _| unreachable!(), &movsx_eax_al).unwrap();
        assert_eq!(bytes32, (-1_i32).to_le_bytes());
        let bytes64 =
            read_source_bytes(&regs, &vectors, |_, _| unreachable!(), &movsx_rax_al).unwrap();
        assert_eq!(bytes64, (-1_i64).to_le_bytes());
        set_register(
            &mut regs,
            Register::EAX,
            u64::from(u32::from_le_bytes(bytes32.try_into().unwrap())),
        )
        .unwrap();
        assert_eq!(regs.value(Register::RAX), Some(0x0000_0000_ffff_ffff));
    }

    #[test]
    fn unrecognised_build_keeps_extracted_slots_and_withholds_unverified_action_bindings() {
        let image = build_synthetic_image(FixtureOptions::default());
        let table = CursorTable::decode_with_anchors(&image, synthetic_anchors(), &[]).unwrap();
        assert_eq!(table.slots.len(), 4);
        assert!(table.bindings.is_empty());
        assert!(
            table
                .path_for(CursorAction::Normal)
                .unwrap_err()
                .contains("not verified for this build")
        );
    }
}
