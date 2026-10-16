use super::*;

/// Native registers before the observed instructions execute.
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
        // DTO + saved flags + eight nonvolatiles + the native CALL return.
        self as *const Self as usize + 208
    }
}
const _: () = assert!(size_of::<Registers>() == 128);

const PAGE_BYTES: usize = 4096;
const UNWIND_OFFSET: usize = 512;
const LEAF_UNWIND_OFFSET: usize = 544;
const COMPLETION_UNWIND_OFFSET: usize = 552;
const TABLE_OFFSET: usize = 560;

/// A CALL frame with registered Win64 unwind information. The complete native
/// instruction span is replaced, and its stack-independent instructions run
/// in a frameless continuation after all native registers have been restored.
/// XMM0..5 survive the callback; the Win64 ABI preserves XMM6..15. Observers
/// use the default SSE2 target and an unwind-permitting Windows ABI.
pub struct Tap {
    pub(super) hook: Hook,
    code: *mut c_void,
}
impl Tap {
    /// # Safety
    /// The caller authenticates the image/sentinel and excludes native return
    /// PCs or incoming branches inside the complete overwritten span. The
    /// native unwind state and handler scope at site+5 must match the site.
    /// Disable and quiesce every executable range before dropping this tap.
    pub unsafe fn new(
        site: usize,
        observer: unsafe extern "system-unwind" fn(&Registers, usize),
        tag: usize,
        sentinel: [u8; 16],
    ) -> Result<Self, String> {
        let instructions = instruction_span(&sentinel)?;
        let code = unsafe {
            VirtualAlloc(
                ptr::null(),
                PAGE_BYTES * 2,
                MEM_RESERVE | MEM_COMMIT,
                PAGE_READWRITE,
            )
        };
        if code.is_null() {
            return Err(io::Error::last_os_error().to_string());
        }
        let result = (|| {
            let mut hook = unsafe { Hook::new(site, code as usize, sentinel) }?;
            hook.patch[0] = 0xe8;
            if instructions == 0 {
                // The two political signal sites
                // end ADD RSP,30; POP RDI; JMP rel32, followed by CC padding.
                // Keep the native epilogue at our CALL return PC. Moving the
                // final jump into the authenticated padding changes no target.
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
            // The adjacent writable page keeps the counter in RIP range. Its
            // lifetime is the bridge's; restoration must prove its flight ended.
            let in_flight = unsafe { &*code.cast::<u8>().add(PAGE_BYTES).cast::<AtomicUsize>() };
            hook.tap = Some(TapSafety {
                in_flight,
                stub: code as usize..code as usize + PAGE_BYTES,
            });
            let bridge = bridge_code(
                observer as *const () as usize,
                tag,
                &sentinel[..instructions],
            );
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
                // The final epilogue JMP [RIP] targets the frameless continuation.
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
                // PUSHFQ is an eight-byte allocation. POPFQ is covered by this
                // record; the following RET has its own frameless record.
                ptr::copy_nonoverlapping(
                    [1_u8, 1, 1, 0, 1, 2, 0, 0].as_ptr(),
                    code.cast::<u8>().add(COMPLETION_UNWIND_OFFSET),
                    8,
                );
                ptr::copy_nonoverlapping(entries.as_ptr(), table(code).cast_mut(), entries.len());
            }
            let mut previous = 0;
            if unsafe { VirtualProtect(code, PAGE_BYTES, PAGE_EXECUTE_READ, &mut previous) } == 0
                || unsafe { FlushInstructionCache(GetCurrentProcess(), code, bridge.code.len()) }
                    == 0
            {
                return Err(io::Error::last_os_error().to_string());
            }
            if !unsafe { RtlAddFunctionTable(table(code), entries.len() as u32, code as u64) } {
                return Err("tap unwind registration failed".into());
            }
            // No fallible operation follows registration. Installation has not
            // published the CALL yet, so construction failures can free safely.
            Ok(Self { hook, code })
        })();
        if result.is_err() {
            unsafe {
                VirtualFree(code, 0, MEM_RELEASE);
            }
        }
        result
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, String> {
        self.hook.set_enabled(enabled)
    }
    pub fn publication_hook(&mut self) -> &mut Hook {
        &mut self.hook
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
        // Hook's Drop also retains its relay/relocation on publication failure.
        // Keep the registered table in this same retained executable page.
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

// These forms cover the political MOV/MOVUPS sites and Experience's MOV+ADD
// prefixes. No copied operand uses RSP, RIP, a branch or a call: the native
// CALL return slot therefore cannot change its meaning. Other forms refuse.
pub fn instruction_span(bytes: &[u8; 16]) -> Result<usize, &'static str> {
    if bytes[..6] == [0x48, 0x83, 0xc4, 0x30, 0x5f, 0xe9] && bytes[10..15] == [0xcc; 5] {
        return Ok(0);
    }
    let mut end = 0;
    while end < 5 {
        if bytes[end] == 0x90 {
            end += 1;
            continue;
        }
        // The reset site's MOV uses REX.R without REX.W.
        let (modrm, rex, sse) = if matches!(bytes[end], 0x44 | 0x48..=0x4f)
            && matches!(bytes[end + 1], 0x8b | 0x89 | 0x03 | 0x01)
        {
            (bytes[end + 2], bytes[end], false)
        } else if bytes[end..end + 2] == [0x0f, 0x11] {
            (bytes[end + 2], 0, true)
        } else {
            return Err("tap instruction form not supported");
        };
        let mode = modrm >> 6;
        let base = modrm & 7;
        let register = (modrm >> 3) & 7;
        if mode == 3 || base == 4 || mode == 0 && base == 5 || !sse && register == 4 && rex & 4 == 0
        {
            return Err("tap stack, register-only or RIP operand not supported");
        }
        end += 3 + match mode {
            0 => 0,
            1 => 1,
            2 => 4,
            _ => unreachable!(),
        };
    }
    Ok(end)
}

struct Bridge {
    code: Vec<u8>,
    unwind: Vec<u8>,
    continuation: usize,
    completion: usize,
    returning: usize,
}

// Win64 UWOP_PUSH_NONVOL / ALLOC_SMALL / SET_FPREG and canonical epilogue:
// https://learn.microsoft.com/en-us/cpp/build/exception-handling-x64
// https://learn.microsoft.com/en-us/cpp/build/prolog-and-epilog
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
    operations.push([code.len() as u8, 2]); // PUSHFQ reserves eight bytes.
    // LEA preserves the native flags that PUSHFQ just saved.
    code.extend([0x48, 0x8d, 0xa4, 0x24, 0x80, 0xff, 0xff, 0xff]);
    operations.push([code.len() as u8, 0xf2]); // 128-byte Registers DTO.
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
    code.extend([0x49, 0x89, 0xe4]); // R12 anchors the fixed frame.
    operations.push([code.len() as u8, 3]);
    let mut unwind = vec![1, code.len() as u8, operations.len() as u8, 12];
    for operation in operations.into_iter().rev() {
        unwind.extend(operation);
    }
    if unwind.len() & 3 != 0 {
        unwind.extend([0, 0]);
    }
    // Interior native sites can have either stack parity. Dynamic allocation
    // outside the prologue unwinds through the established frame register.
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
    flight_instruction(&mut code, 0x05); // LOCK INC, after saving native flags.
    code.push(0xfc); // Observers run with DF clear; saved native flags survive.
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
    // R12 remains live across this temporary PUSH, so both boundaries unwind.
    code.extend([0x41, 0xff, 0xb4, 0x24, 0x80, 0, 0, 0, 0x9d]);
    // Skip the DTO and saved flags, then pop only contiguous nonvolatiles.
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
    code.extend(instructions);
    let completion = code.len();
    code.push(0x9c);
    flight_instruction(&mut code, 0x0d); // LOCK DEC only after copied instructions.
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
