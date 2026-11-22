use super::*;

#[repr(C)]
pub struct Registers {
    pub r15: usize,
    pub r14: usize,
    pub r13: usize,
    pub r12: usize,
    pub r11: usize,
    pub r10: usize,
    pub r9: usize,
    pub r8: usize,
    pub rdi: usize,
    pub rsi: usize,
    pub rbp: usize,
    pub rbx: usize,
    pub rdx: usize,
    pub rcx: usize,
    pub rax: usize,
    pub flags: usize,
}
#[cfg(any(test, feature = "test-support"))]
impl Registers {
    pub fn rsp(&self) -> usize {
        self as *const Self as usize + CALL_RETURN_OFFSET + size_of::<usize>()
    }
}
const _: () = assert!(size_of::<Registers>() == 128);
const CALL_RETURN_OFFSET: usize = size_of::<Registers>() + 9 * size_of::<usize>();

const PAGE_BYTES: usize = 4096;
const UNWIND_OFFSET: usize = 512;
const LEAF_UNWIND_OFFSET: usize = 544;
const COMPLETION_UNWIND_OFFSET: usize = 552;
const TABLE_OFFSET: usize = 560;

struct JumpTail {
    observer: unsafe extern "system-unwind" fn(&Registers, usize),
    tag: usize,
    target: usize,
}
unsafe extern "system-unwind" fn jump_tail(registers: &Registers, tag: usize) {
    let binding = unsafe { &*(tag as *const JumpTail) };
    let return_slot = (registers as *const Registers as usize + CALL_RETURN_OFFSET) as *mut usize;
    unsafe {
        ptr::write(return_slot, binding.target);
        (binding.observer)(registers, binding.tag);
    }
}

fn has_jump_tail(bytes: &[u8; 16]) -> bool {
    bytes[..5] == [0x44, 0x89, 0x63, 0x28, 0xe9] || bytes[..4] == [0x89, 0x43, 0x30, 0xe9]
}

pub struct Tap {
    pub(super) hook: Hook,
    code: *mut c_void,
}
impl Tap {
    /// Publishes and verifies only the complete native instructions this tap replaces.
    /// Adjacent taps may share untouched tails of their construction sentinels.
    ///
    /// # Safety
    /// The caller must authenticate all 16 sentinel bytes before construction and satisfy
    /// every requirement of [`Self::new`]. Other patches must not overlap the replaced span.
    pub unsafe fn new_owned_span(
        site: usize,
        observer: unsafe extern "system-unwind" fn(&Registers, usize),
        tag: usize,
        sentinel: [u8; 16],
    ) -> Result<Self, String> {
        let mut tap = unsafe { Self::new(site, observer, tag, sentinel) }?;
        tap.hook.sentinel_len = tap.hook.patch_len;
        Ok(tap)
    }

    /// # Safety
    /// The caller authenticates the image/sentinel and excludes native return
    /// PCs or incoming branches inside the complete overwritten span. The
    /// native unwind state and handler scope at the five-byte return point must match the site.
    /// A lifecycle jump tail also requires its target to have that same native frame and handler state.
    /// Every call of `observer` with the site's registers and `tag` must meet the observer's own
    /// preconditions, so any storage `tag` refers to stays valid until callbacks have quiesced.
    /// Disable and quiesce every executable range before dropping this tap.
    pub unsafe fn new(
        site: usize,
        observer: unsafe extern "system-unwind" fn(&Registers, usize),
        tag: usize,
        sentinel: [u8; 16],
    ) -> Result<Self, String> {
        super::installation::construct(site, || {
            let instructions = instruction_span(&sentinel)?;
            let jump_target = if has_jump_tail(&sentinel) {
                let mut policy = 0_u32;
                if unsafe {
                    GetProcessMitigationPolicy(
                        GetCurrentProcess(),
                        ProcessUserShadowStackPolicy,
                        (&mut policy as *mut u32).cast(),
                        size_of::<u32>(),
                    )
                } == 0
                {
                    return Err(format!(
                        "tap shadow-stack policy unreadable: {}",
                        io::Error::last_os_error()
                    ));
                }
                if policy & 1 != 0 {
                    return Err(
                        "tap native jump continuation requires disabled user shadow stack".into(),
                    );
                }
                let store = if sentinel[0] == 0x44 { 4 } else { 3 };
                let end = store + 5;
                let displacement = i32::from_le_bytes(sentinel[store + 1..end].try_into().unwrap());
                Some(
                    site.checked_add(end)
                        .and_then(|at| at.checked_add_signed(displacement as isize))
                        .ok_or("tap native jump target overflow")?,
                )
            } else {
                None
            };
            let store_target = rip_store_target(site, &sentinel[..instructions])?;
            let code = if let Some(target) = store_target {
                super::allocate_near(target, PAGE_BYTES * 2)
            } else {
                unsafe {
                    VirtualAlloc(
                        ptr::null(),
                        PAGE_BYTES * 2,
                        MEM_RESERVE | MEM_COMMIT,
                        PAGE_READWRITE,
                    )
                }
            };
            if code.is_null() {
                return Err(io::Error::last_os_error().to_string());
            }
            let result = (|| {
                let owned_span = if instructions == 0 { 15 } else { instructions };
                let mut hook =
                    unsafe { Hook::new_span(site, code as usize, sentinel, owned_span) }?;
                hook.patch[0] = 0xe8;
                if instructions == 0 {
                    hook.patch[5..10].copy_from_slice(&sentinel[..5]);
                    hook.patch[10] = 0xe9;
                    let displacement = i32::from_le_bytes(sentinel[6..10].try_into().unwrap())
                        .checked_sub(5)
                        .ok_or("tap epilogue displacement overflow")?;
                    hook.patch[11..15].copy_from_slice(&displacement.to_le_bytes());
                    hook.patch_len = 15;
                } else {
                    hook.patch[5..instructions].fill(0x90);
                    hook.patch_len = instructions;
                }
                let in_flight =
                    unsafe { &*code.cast::<u8>().add(PAGE_BYTES).cast::<AtomicUsize>() };
                hook.tap = Some(TapSafety {
                    in_flight,
                    stub: code as usize..code as usize + PAGE_BYTES,
                });
                let mut bridge = if let Some(target) = jump_target {
                    let binding = unsafe {
                        code.cast::<u8>()
                            .add(PAGE_BYTES + size_of::<AtomicUsize>())
                            .cast::<JumpTail>()
                    };
                    unsafe {
                        ptr::write(
                            binding,
                            JumpTail {
                                observer,
                                tag,
                                target,
                            },
                        );
                    }
                    bridge_code(
                        jump_tail as *const () as usize,
                        binding as usize,
                        &sentinel[..if sentinel[0] == 0x44 { 4 } else { 3 }],
                    )
                } else {
                    bridge_code(
                        observer as *const () as usize,
                        tag,
                        &sentinel[..instructions],
                    )
                };
                if let Some(target) = store_target {
                    relocate_rip_store(
                        &mut bridge.code[bridge.continuation..bridge.completion],
                        code as usize + bridge.continuation,
                        target,
                    )?;
                }
                if bridge.code.len() > UNWIND_OFFSET
                    || bridge.unwind.len() > LEAF_UNWIND_OFFSET - UNWIND_OFFSET
                {
                    return Err("tap code and unwind storage overlap".into());
                }
                let mut entries = [
                    entry(0, bridge.continuation, UNWIND_OFFSET),
                    entry(bridge.continuation, bridge.completion, LEAF_UNWIND_OFFSET),
                    entry(
                        bridge.completion,
                        bridge.returning,
                        COMPLETION_UNWIND_OFFSET,
                    ),
                    entry(bridge.returning, bridge.code.len(), LEAF_UNWIND_OFFSET),
                ];
                let count = if bridge.continuation == bridge.completion {
                    entries[1] = entries[2];
                    entries[2] = entries[3];
                    3
                } else {
                    4
                };
                let entries = &entries[..count];
                unsafe {
                    ptr::copy_nonoverlapping(bridge.code.as_ptr(), code.cast(), bridge.code.len());
                    ptr::write_unaligned(
                        code.cast::<u8>().add(bridge.continuation - 8).cast::<u64>(),
                        code as u64 + bridge.continuation as u64,
                    );
                    ptr::copy_nonoverlapping(
                        bridge.unwind.as_ptr(),
                        code.cast::<u8>().add(UNWIND_OFFSET),
                        bridge.unwind.len(),
                    );
                    ptr::copy_nonoverlapping(
                        [1_u8, 0, 0, 0].as_ptr(),
                        code.cast::<u8>().add(LEAF_UNWIND_OFFSET),
                        4,
                    );
                    ptr::copy_nonoverlapping(
                        [1_u8, 1, 1, 0, 1, 2, 0, 0].as_ptr(),
                        code.cast::<u8>().add(COMPLETION_UNWIND_OFFSET),
                        8,
                    );
                    ptr::copy_nonoverlapping(
                        entries.as_ptr(),
                        table(code).cast_mut(),
                        entries.len(),
                    );
                }
                let mut previous = 0;
                if unsafe { VirtualProtect(code, PAGE_BYTES, PAGE_EXECUTE_READ, &mut previous) }
                    == 0
                    || unsafe {
                        FlushInstructionCache(GetCurrentProcess(), code, bridge.code.len())
                    } == 0
                {
                    return Err(io::Error::last_os_error().to_string());
                }
                if !unsafe { RtlAddFunctionTable(table(code), entries.len() as u32, code as u64) } {
                    return Err("tap unwind registration failed".into());
                }
                Ok(Self { hook, code })
            })();
            if result.is_err() {
                unsafe {
                    VirtualFree(code, 0, MEM_RELEASE);
                }
            }
            result
        })
    }
    pub fn named(mut self, name: &'static str) -> Self {
        self.hook.name = name;
        self
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, String> {
        self.hook.set_enabled(enabled)
    }
    pub fn publication_hook(&mut self) -> &mut Hook {
        &mut self.hook
    }
    pub fn is_enabled(&self) -> bool {
        self.hook.is_enabled()
    }
    pub fn is_published(&self) -> bool {
        self.hook.is_published()
    }
    pub fn executable_ranges(&self) -> Vec<Range<usize>> {
        self.hook
            .executable_ranges()
            .into_iter()
            .chain(std::iter::once(
                self.code as usize..self.code as usize + PAGE_BYTES,
            ))
            .collect()
    }
}
impl Drop for Tap {
    fn drop(&mut self) {
        if self.hook.is_enabled() || !unregister(self.code) {
            return;
        }
        unsafe {
            VirtualFree(self.code, 0, MEM_RELEASE);
        }
    }
}

fn table(code: *mut c_void) -> *const IMAGE_RUNTIME_FUNCTION_ENTRY {
    unsafe { code.cast::<u8>().add(TABLE_OFFSET).cast() }
}
fn unregister(code: *mut c_void) -> bool {
    #[cfg(any(test, feature = "test-support"))]
    if FAIL_UNREGISTER.with(|fail| fail.replace(false)) {
        return false;
    }
    unsafe { RtlDeleteFunctionTable(table(code)) }
}
#[cfg(any(test, feature = "test-support"))]
std::thread_local! { static FAIL_UNREGISTER: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }

fn entry(begin: usize, end: usize, unwind: usize) -> IMAGE_RUNTIME_FUNCTION_ENTRY {
    IMAGE_RUNTIME_FUNCTION_ENTRY {
        BeginAddress: begin as u32,
        EndAddress: end as u32,
        Anonymous: IMAGE_RUNTIME_FUNCTION_ENTRY_0 {
            UnwindInfoAddress: unwind as u32,
        },
    }
}

pub fn instruction_span(bytes: &[u8; 16]) -> Result<usize, &'static str> {
    if bytes.starts_with(&[0xc7, 0x05]) {
        return Ok(10);
    }
    if counter_reload(bytes) {
        if i32::from_le_bytes(bytes[6..10].try_into().unwrap())
            .checked_add(8)
            .is_none()
        {
            return Err("tap counter reload displacement overflow");
        }
        return Ok(10);
    }
    for (prefix, span) in [
        (&[0x85, 0xc0, 0x0f, 0x44, 0xc3][..], 5),
        (&[0x85, 0xc0, 0x41, 0x0f, 0x44, 0xc7][..], 6),
        (&[0x41, 0x8b, 0xfc, 0x85, 0xed][..], 5),
        (&[0x41, 0x8b, 0xff, 0x8b, 0x75, 0x64][..], 6),
        (&[0x41, 0x0f, 0xb6, 0x46, 0x48, 0x88, 0x41, 0x2c][..], 8),
    ] {
        if bytes.starts_with(prefix) {
            return Ok(span);
        }
    }
    if bytes[..7] == [0x41, 0x89, 0x86, 0x6c, 1, 0, 0] {
        return Ok(7);
    }
    if has_jump_tail(bytes) {
        return Ok(if bytes[0] == 0x44 { 9 } else { 8 });
    }
    if bytes[..6] == [0x48, 0x83, 0xc4, 0x30, 0x5f, 0xe9] && bytes[10..15] == [0xcc; 5] {
        return Ok(0);
    }
    let mut end = 0;
    while end < 5 {
        if bytes[end..end + 2] == [0x8b, 0xd0] || bytes[end..end + 3] == [0x8b, 0x53, 0x78] {
            end += if bytes[end] == 0x8b && bytes[end + 1] == 0xd0 {
                2
            } else {
                3
            };
            continue;
        }
        if matches!(
            &bytes[end..end + 3],
            [0x89, 0x4b, 0x28] | [0x89, 0x43, 0x30]
        ) {
            end += 3;
            continue;
        }
        if bytes[end..end + 4] == [0x48, 0x83, 0xc2, 0x08] {
            end += 4;
            continue;
        }
        if bytes[end..end + 6] == [0x41, 0xbf, 1, 0, 0, 0] {
            end += 6;
            continue;
        }
        if bytes[end..end + 5] == [0xba, 1, 0, 0, 0] {
            end += 5;
            continue;
        }
        if bytes[end..end + 7] == [0xc7, 0x43, 0x28, 0, 0, 0, 0]
            || bytes[end..end + 7] == [0xc7, 0x47, 0x1c, 0, 0, 0, 0]
            || bytes[end..end + 7] == [0xc7, 0x43, 0x1c, 4, 0, 0, 0]
        {
            end += 7;
            continue;
        }
        if bytes[end..end + 6] == [0xff, 0x8e, 0x38, 0x05, 0, 0] {
            end += 6;
            continue;
        }
        if bytes[end] == 0x90 {
            end += 1;
            continue;
        }
        if bytes[end..end + 3] == [0x45, 0x33, 0xf6] {
            end += 3;
            continue;
        }
        let rex = if matches!(bytes[end], 0x40..=0x4f) {
            bytes[end]
        } else {
            0
        };
        let prefix = usize::from(rex != 0);
        let opcode = bytes[end + prefix];
        let register = |low: u8, extension: u8| low != 4 || rex & extension != 0;
        if matches!(opcode, 0xb8..=0xbf) {
            if !register(opcode & 7, 1) {
                return Err("tap stack register immediate not supported");
            }
            end += prefix + if rex & 8 != 0 { 9 } else { 5 };
            continue;
        }
        if opcode == 0x0f && matches!(bytes[end + prefix + 1], 0x90..=0x9f) {
            let modrm = bytes[end + prefix + 2];
            if modrm >> 6 != 3 || modrm & 0x38 != 0 || (rex != 0 && modrm & 7 == 4 && rex & 1 == 0)
            {
                return Err("tap condition result requires a stack-independent byte register");
            }
            end += prefix + 3;
            continue;
        }
        let conditional = opcode == 0x0f && matches!(bytes[end + prefix + 1], 0x48 | 0x4f);
        if matches!(opcode, 0x31 | 0x33) {
            let modrm = bytes[end + prefix + 1];
            if modrm >> 6 != 3 || !register((modrm >> 3) & 7, 4) || !register(modrm & 7, 1) {
                return Err("tap XOR requires stack-independent registers");
            }
            end += prefix + 2;
            continue;
        }
        if conditional || opcode == 0x3b {
            let modrm = bytes[end + prefix + 1 + usize::from(conditional)];
            if modrm >> 6 != 3 || !register((modrm >> 3) & 7, 4) || !register(modrm & 7, 1) {
                return Err("tap clamp or comparison is not stack-independent registers");
            }
            end += prefix + 2 + usize::from(conditional);
            continue;
        }
        if bytes[end..end + 3] == [0x45, 0x33, 0xf6] {
            end += 3;
            continue;
        }
        let mut cursor = end;
        let rex = if matches!(bytes[cursor], 0x40..=0x4f) {
            let rex = bytes[cursor];
            cursor += 1;
            rex
        } else {
            0
        };
        let opcode = bytes[cursor];
        cursor += 1;
        let sse = opcode == 0x0f && bytes.get(cursor) == Some(&0x11);
        if sse {
            cursor += 1;
        } else if !matches!(
            opcode,
            0x88 | 0x89
                | 0x8b
                | 0x8d
                | 0x03
                | 0x01
                | 0x29
                | 0x2b
                | 0x63
                | 0x85
                | 0x80
                | 0x83
                | 0xc6
                | 0xc7
                | 0xff
        ) {
            return Err("tap instruction form not supported");
        }
        let modrm = *bytes.get(cursor).ok_or("tap instruction truncated")?;
        cursor += 1;
        let mode = modrm >> 6;
        let base = modrm & 7;
        let register = (modrm >> 3) & 7;
        let immediate = match opcode {
            0x80 if register == 7 => 1,
            0x83 if matches!(register, 0 | 5 | 7) => 1,
            0xc6 if register == 0 => 1,
            0xc7 if register == 0 => 4,
            0xff if matches!(register, 0 | 1) => 0,
            0x80 | 0x83 | 0xc6 | 0xc7 | 0xff => return Err("tap opcode extension not supported"),
            _ => 0,
        };
        let explicit_register = !sse && !matches!(opcode, 0x80 | 0x83 | 0xc6 | 0xc7 | 0xff);
        if explicit_register && register == 4 && rex & 4 == 0 && (opcode != 0x88 || rex != 0) {
            return Err("tap stack register not supported");
        }
        if mode == 3 {
            if !matches!(
                opcode,
                0x89 | 0x8b | 0x03 | 0x01 | 0x29 | 0x2b | 0x63 | 0x85 | 0x80 | 0x83 | 0xff
            ) || base == 4 && rex & 1 == 0
            {
                return Err("tap register-only operand not supported");
            }
        } else {
            if mode == 0 && base == 5 {
                return Err("tap SIB, stack or RIP operand not supported");
            }
            if base == 4 && opcode == 0x8d {
                return Err("tap SIB LEA operand not supported");
            }
            if base == 4 {
                let sib = *bytes.get(cursor).ok_or("tap instruction truncated")?;
                cursor += 1;
                let sib_base = sib & 7;
                if sib_base == 4 && rex & 1 == 0 || mode == 0 && sib_base == 5 {
                    return Err("tap stack or unbased SIB operand not supported");
                }
            }
            cursor += match mode {
                0 => 0,
                1 => 1,
                2 => 4,
                _ => unreachable!(),
            };
        }
        cursor += immediate;
        if cursor > bytes.len() {
            return Err("tap instruction truncated");
        }
        end = cursor;
    }
    Ok(end)
}

fn counter_reload(bytes: &[u8]) -> bool {
    bytes.len() >= 10 && bytes[..2] == [0xff, 0x47] && bytes[3..6] == [0x8b, 0xbc, 0x24]
}

fn rip_store_target(site: usize, instructions: &[u8]) -> Result<Option<usize>, &'static str> {
    if !instructions.starts_with(&[0xc7, 0x05]) {
        return Ok(None);
    }
    if instructions.len() != 10 {
        return Err("tap RIP store has an invalid span");
    }
    let displacement = i32::from_le_bytes(instructions[2..6].try_into().unwrap());
    site.checked_add(10)
        .and_then(|end| end.checked_add_signed(displacement as isize))
        .map(Some)
        .ok_or("tap RIP store target overflow")
}

fn relocate_rip_store(
    instructions: &mut [u8],
    at: usize,
    target: usize,
) -> Result<(), &'static str> {
    if instructions.len() != 10 || !instructions.starts_with(&[0xc7, 0x05]) {
        return Err("tap RIP store has an invalid continuation");
    }
    let end = at.checked_add(10).ok_or("tap RIP continuation overflow")?;
    let displacement = i32::try_from(target as i128 - end as i128)
        .map_err(|_| "tap RIP store target is out of range")?;
    instructions[2..6].copy_from_slice(&displacement.to_le_bytes());
    Ok(())
}

struct Bridge {
    code: Vec<u8>,
    unwind: Vec<u8>,
    continuation: usize,
    completion: usize,
    returning: usize,
}

fn bridge_code(observer: usize, tag: usize, instructions: &[u8]) -> Bridge {
    let mut code = Vec::with_capacity(512);
    let mut operations = Vec::with_capacity(11);
    for register in [5_u8, 3, 6, 7, 12, 13, 14, 15] {
        if register >= 8 {
            code.push(0x41);
        }
        code.push(0x50 | (register & 7));
        operations.push([code.len() as u8, register << 4]);
    }
    code.push(0x9c);
    operations.push([code.len() as u8, 2]);
    code.extend([0x48, 0x8d, 0xa4, 0x24, 0x80, 0xff, 0xff, 0xff]);
    operations.push([code.len() as u8, 0xf2]);
    for (slot, register) in [15_u8, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 3, 2, 1, 0]
        .into_iter()
        .enumerate()
    {
        code.extend([
            0x48 | if register >= 8 { 4 } else { 0 },
            0x89,
            0x44 | ((register & 7) << 3),
            0x24,
            slot as u8 * 8,
        ]);
    }
    code.extend([0x48, 0x8b, 0x84, 0x24, 0x80, 0, 0, 0]);
    code.extend([0x48, 0x89, 0x44, 0x24, 0x78]);
    code.extend([0x49, 0x89, 0xe4]);
    operations.push([code.len() as u8, 3]);
    let mut unwind = vec![1, code.len() as u8, operations.len() as u8, 12];
    for operation in operations.into_iter().rev() {
        unwind.extend(operation);
    }
    if unwind.len() & 3 != 0 {
        unwind.extend([0, 0]);
    }
    code.extend([0x48, 0x83, 0xe4, 0xf0, 0x48, 0x81, 0xec, 0x80, 0, 0, 0]);
    for register in 0..6 {
        code.extend([
            0xf3,
            0x0f,
            0x7f,
            0x44 | (register << 3),
            0x24,
            0x20 + register * 16,
        ]);
    }
    flight_instruction(&mut code, 0x05);
    code.push(0xfc);
    code.extend([0x4c, 0x89, 0xe1, 0x48, 0xba]);
    code.extend((tag as u64).to_le_bytes());
    code.extend([0x48, 0xb8]);
    code.extend((observer as u64).to_le_bytes());
    code.extend([0xff, 0xd0]);
    for register in 0..6 {
        code.extend([
            0xf3,
            0x0f,
            0x6f,
            0x44 | (register << 3),
            0x24,
            0x20 + register * 16,
        ]);
    }
    for (register, offset) in [
        (0, 112),
        (1, 104),
        (2, 96),
        (8, 56),
        (9, 48),
        (10, 40),
        (11, 32),
    ] {
        code.extend([
            0x49 | if register >= 8 { 4 } else { 0 },
            0x8b,
            0x44 | ((register & 7) << 3),
            0x24,
            offset,
        ]);
    }
    code.extend([0x41, 0xff, 0xb4, 0x24, 0x80, 0, 0, 0, 0x9d]);
    code.extend([0x49, 0x8d, 0xa4, 0x24, 0x88, 0, 0, 0]);
    for register in [15_u8, 14, 13, 12, 7, 6, 3, 5] {
        if register >= 8 {
            code.push(0x41);
        }
        code.push(0x58 | (register & 7));
    }
    code.extend([0xff, 0x25, 0, 0, 0, 0]);
    code.extend([0; 8]);
    let continuation = code.len();
    if counter_reload(instructions) {
        code.extend(&instructions[..6]);
        let displacement = i32::from_le_bytes(instructions[6..10].try_into().unwrap());
        code.extend(
            displacement
                .checked_add(8)
                .expect("verified counter reload displacement")
                .to_le_bytes(),
        );
    } else {
        code.extend(instructions);
    }
    let completion = code.len();
    code.push(0x9c);
    flight_instruction(&mut code, 0x0d);
    code.push(0x9d);
    let returning = code.len();
    code.push(0xc3);
    Bridge {
        code,
        unwind,
        continuation,
        completion,
        returning,
    }
}

fn flight_instruction(code: &mut Vec<u8>, modrm: u8) {
    let displacement = (PAGE_BYTES - (code.len() + 8)) as i32;
    code.extend([0xf0, 0x48, 0xff, modrm]);
    code.extend(displacement.to_le_bytes());
}

#[cfg(test)]
#[path = "tap/tests.rs"]
mod tests;
