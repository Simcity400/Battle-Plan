use super::*;

use std::sync::atomic::{AtomicUsize, Ordering};
use windows_sys::Win32::System::Diagnostics::Debug::{
    RtlCaptureContext, RtlLookupFunctionEntry, RtlVirtualUnwind,
};

static SEEN: AtomicUsize = AtomicUsize::new(0);
static FAILURES: AtomicUsize = AtomicUsize::new(0);

#[test]
fn world_invalidation_mov_spans_preserve_the_native_stack_and_operands() {
    assert_eq!(
        instruction_span(&engine::world_lifetime_sites::RESET.sentinel),
        Ok(7)
    );
    assert_eq!(
        instruction_span(&engine::world_lifetime_sites::CONSTRUCT.sentinel),
        Ok(7)
    );
}

#[test]
fn directed_war_query_spans_copy_only_the_native_pointer_load() {
    for site in engine::directed_war_queries::SITES {
        assert_eq!(instruction_span(&site.sentinel), Ok(7));
        assert_eq!(&site.sentinel[3..7], &[0xe8, 2, 0, 0]);
    }
}

fn unwind(context: &mut CONTEXT) -> bool {
    let mut aligned = AlignedContext(*context);
    let mut base = 0;
    let entry = unsafe { RtlLookupFunctionEntry(aligned.0.Rip, &mut base, ptr::null_mut()) };
    if entry.is_null() {
        return false;
    }
    let mut data = ptr::null_mut();
    let mut frame = 0;
    unsafe {
        RtlVirtualUnwind(
            0,
            base,
            aligned.0.Rip,
            entry,
            &mut aligned.0,
            &mut data,
            &mut frame,
            ptr::null_mut(),
        );
    }
    *context = aligned.0;
    true
}
fn lookup(address: usize) -> bool {
    let mut base = 0;
    !unsafe { RtlLookupFunctionEntry(address as u64, &mut base, ptr::null_mut()) }.is_null()
}

unsafe extern "system-unwind" fn observer(registers: &Registers, tag: usize) {
    let mut failure = 0;
    if registers.rcx != 0x1234 || registers.rdx != 0x5678 || tag != 73 {
        failure |= 1;
    }
    let native_rsp = registers.rsp();
    let native_pc = unsafe { ptr::read((native_rsp - 8) as *const u64) };
    let mut context = AlignedContext(unsafe { std::mem::zeroed() });
    unsafe {
        RtlCaptureContext(&mut context.0);
    }
    // First leave this real Rust callback, then unwind its generated caller.
    if !unwind(&mut context.0) || !unwind(&mut context.0) {
        failure |= 2;
    } else if context.0.Rip != native_pc
        || context.0.Rsp != native_rsp as u64
        || [
            context.0.Rbx,
            context.0.Rbp,
            context.0.Rsi,
            context.0.Rdi,
            context.0.R12,
            context.0.R13,
            context.0.R14,
            context.0.R15,
        ] != [
            registers.rbx as u64,
            registers.rbp as u64,
            registers.rsi as u64,
            registers.rdi as u64,
            registers.r12 as u64,
            registers.r13 as u64,
            registers.r14 as u64,
            registers.r15 as u64,
        ]
    {
        failure |= 4;
    }
    FAILURES.fetch_or(failure, Ordering::Relaxed);
    SEEN.fetch_add(1, Ordering::Relaxed);
    unsafe {
        core::arch::asm!(
            "pxor xmm0, xmm0", "pxor xmm1, xmm1", "pxor xmm2, xmm2",
            "pxor xmm3, xmm3", "pxor xmm4, xmm4", "pxor xmm5, xmm5",
            out("xmm0") _, out("xmm1") _, out("xmm2") _,
            out("xmm3") _, out("xmm4") _, out("xmm5") _,
        );
    }
}
#[unsafe(naked)]
unsafe extern "system" fn invoke(entry: usize, a: usize, b: usize, sink: *mut u64) -> u64 {
    core::arch::naked_asm!(
        "mov r10, rcx",
        "mov rcx, rdx",
        "mov rdx, r8",
        "mov rax, 0x1111",
        "movq xmm0, rax",
        "stc",
        "jmp r10",
    );
}
unsafe extern "system-unwind" fn idle(_: &Registers, _: usize) {}

struct NativeTable(*const IMAGE_RUNTIME_FUNCTION_ENTRY);
impl Drop for NativeTable {
    fn drop(&mut self) {
        unsafe {
            RtlDeleteFunctionTable(self.0);
        }
    }
}

fn stub(code: &TestCode, aligned: bool) -> (usize, [u8; 16]) {
    let mut bytes = Vec::<u8>::new();
    if aligned {
        bytes.extend([0x48, 0x8d, 0x64, 0x24, 0xd8]); // LEA keeps the caller's CF.
    }
    let site = code.0 as usize + bytes.len();
    // Independently authored MOV [R9+128],RCX. Capture CF before arithmetic.
    bytes.extend([0x49, 0x89, 0x89, 0x80, 0, 0, 0, 0x9c, 0x41, 0x5a]);
    bytes.extend([0x4d, 0x89, 0x91, 0x88, 0, 0, 0]);
    bytes.extend([
        0x66, 0x48, 0x0f, 0x7e, 0xc0, 0x48, 0x01, 0xc8, 0x48, 0x01, 0xd0,
    ]);
    if aligned {
        bytes.extend([0x48, 0x8d, 0x64, 0x24, 0x28]);
    }
    bytes.push(0xc3);
    unsafe {
        ptr::copy_nonoverlapping(bytes.as_ptr(), code.0.cast(), bytes.len());
    }
    let sentinel = unsafe { std::slice::from_raw_parts(site as *const u8, 16) }
        .try_into()
        .unwrap();
    (site, sentinel)
}

#[test]
fn callback_unwinds_to_native_continuation_and_preserves_registers_flags_and_both_parities() {
    let _guard = FAMILY_TEST.lock().unwrap();
    SEEN.store(0, Ordering::Relaxed);
    FAILURES.store(0, Ordering::Relaxed);
    for aligned in [false, true] {
        let code = TestCode::new();
        let (site, sentinel) = stub(&code, aligned);
        let mut tap = unsafe { Tap::new(site, observer, 73, sentinel) }.unwrap();
        assert!(tap.set_enabled(true).unwrap());
        let mut sink = [0_u64; 18];
        assert_eq!(
            unsafe { invoke(code.0 as usize, 0x1234, 0x5678, sink.as_mut_ptr()) },
            0x1111 + 0x1234 + 0x5678
        );
        assert_eq!(sink[16], 0x1234);
        assert_eq!(sink[17] & 1, 1);
        assert!(tap.set_enabled(false).unwrap());
        assert_eq!(
            unsafe { std::slice::from_raw_parts(site as *const u8, 16) },
            sentinel
        );
        let address = tap.code as usize;
        assert!(lookup(address));
        drop(tap);
        assert!(!lookup(address));
    }
    assert_eq!(SEEN.load(Ordering::Relaxed), 2);
    assert_eq!(FAILURES.load(Ordering::Relaxed), 0);
}

fn nonvolatile(context: &CONTEXT) -> [u64; 8] {
    [
        context.R15,
        context.R14,
        context.R13,
        context.R12,
        context.Rdi,
        context.Rsi,
        context.Rbx,
        context.Rbp,
    ]
}
fn set_nonvolatile(context: &mut CONTEXT, values: [u64; 8]) {
    [
        context.R15,
        context.R14,
        context.R13,
        context.R12,
        context.Rdi,
        context.Rsi,
        context.Rbx,
        context.Rbp,
    ] = values;
}

#[test]
fn virtual_unwind_covers_partial_prologue_dynamic_frame_and_every_epilogue_boundary() {
    let _guard = FAMILY_TEST.lock().unwrap();
    let code = TestCode::new();
    let (site, sentinel) = stub(&code, false);
    let tap = unsafe { Tap::new(site, idle, 0, sentinel) }.unwrap();
    let bridge = bridge_code(idle as *const () as usize, 0, &sentinel[..7]);
    let epilogue = bridge
        .code
        .windows(8)
        .position(|bytes| bytes == [0x49, 0x8d, 0xa4, 0x24, 0x88, 0, 0, 0])
        .unwrap();
    let saved = [115_u64, 114, 113, 112, 107, 106, 103, 105];
    let mut stack = [0_u64; 256];
    let native_rsp = unsafe { stack.as_mut_ptr().add(240) } as usize;
    let return_pc = 0x1234_5678;
    unsafe {
        ptr::write((native_rsp - 8) as *mut u64, return_pc);
    }
    for (index, value) in saved.iter().enumerate() {
        unsafe {
            ptr::write((native_rsp - 72 + index * 8) as *mut u64, *value);
        }
    }
    let fixed = native_rsp - 208;
    let mut context = CONTEXT {
        Rip: tap.code as u64,
        Rsp: native_rsp as u64 - 8,
        ..Default::default()
    };
    set_nonvolatile(&mut context, saved);
    let mut copy = context;
    assert!(unwind(&mut copy));
    assert_eq!(
        (copy.Rip, copy.Rsp, nonvolatile(&copy)),
        (return_pc, native_rsp as u64, saved)
    );
    // Real instruction offsets, independent of the emitted unwind-code array.
    let pushes = [
        (1, 7),
        (2, 6),
        (3, 5),
        (4, 4),
        (6, 3),
        (8, 2),
        (10, 1),
        (12, 0),
    ];
    for (offset, register) in pushes {
        context.Rip = tap.code as u64 + offset;
        context.Rsp -= 8;
        let mut copy = context;
        assert!(unwind(&mut copy));
        assert_eq!(
            (copy.Rip, copy.Rsp, nonvolatile(&copy)),
            (return_pc, native_rsp as u64, saved)
        );
        assert_eq!(nonvolatile(&context)[register], saved[register]);
    }
    for (offset, rsp, frame) in [
        (13, native_rsp - 80, false),
        (21, fixed, false),
        (112, fixed, true),
        (123, (fixed & !15) - 128, true),
        (epilogue as u64 - 9, (fixed & !15) - 128, true),
        (epilogue as u64 - 1, (fixed & !15) - 136, true),
    ] {
        context.Rip = tap.code as u64 + offset;
        context.Rsp = rsp as u64;
        context.R12 = if frame { fixed as u64 } else { saved[3] };
        let mut copy = context;
        assert!(unwind(&mut copy));
        assert_eq!(
            (copy.Rip, copy.Rsp, nonvolatile(&copy)),
            (return_pc, native_rsp as u64, saved)
        );
    }
    context.R12 = fixed as u64;
    context.Rsp = ((fixed & !15) - 128) as u64;
    context.Rip = tap.code as u64 + epilogue as u64;
    let mut copy = context;
    assert!(unwind(&mut copy));
    assert_eq!(
        (copy.Rip, copy.Rsp, nonvolatile(&copy)),
        (return_pc, native_rsp as u64, saved)
    );
    context.Rsp = (native_rsp - 72) as u64;
    context.Rip += 8;
    for (index, value) in saved.into_iter().enumerate() {
        let mut copy = context;
        assert!(unwind(&mut copy));
        assert_eq!(
            (copy.Rip, copy.Rsp, nonvolatile(&copy)),
            (return_pc, native_rsp as u64, saved)
        );
        let mut values = nonvolatile(&context);
        values[index] = value;
        set_nonvolatile(&mut context, values);
        context.Rsp += 8;
        context.Rip += if index < 4 { 2 } else { 1 };
    }
    for offset in [context.Rip, tap.code as u64 + bridge.continuation as u64] {
        let mut copy = context;
        copy.Rip = offset;
        assert!(unwind(&mut copy));
        assert_eq!(
            (copy.Rip, copy.Rsp, nonvolatile(&copy)),
            (return_pc, native_rsp as u64, saved)
        );
    }
}

#[test]
fn refused_publication_or_unregister_retains_registered_executable_storage() {
    let _guard = FAMILY_TEST.lock().unwrap();
    for mode in [0, 1, 2] {
        let code = TestCode::new();
        let (site, sentinel) = stub(&code, false);
        let mut tap = unsafe { Tap::new(site, idle, 0, sentinel) }.unwrap();
        let address = tap.code;
        match mode {
            0 => {
                assert!(tap.set_enabled(true).unwrap());
            }
            1 => {
                fail_flushes(2);
                assert!(
                    tap.set_enabled(true)
                        .unwrap_err()
                        .contains("restored bytes remain unflushed")
                );
                assert!(!tap.hook.enabled && tap.hook.is_enabled());
            }
            _ => FAIL_UNREGISTER.with(|fail| fail.set(true)),
        }
        drop(tap);
        assert!(lookup(address as usize));
        // Deliberate invalid-owner drops are confined to synthetic code. Restore
        // its bytes and remove the retained table before releasing either page.
        unsafe {
            ptr::copy_nonoverlapping(sentinel.as_ptr(), site as *mut u8, 16);
        }
        assert_ne!(
            unsafe { FlushInstructionCache(GetCurrentProcess(), site as *const c_void, 16) },
            0
        );
        assert!(unregister(address));
        assert!(!lookup(address as usize));
        assert_ne!(unsafe { VirtualFree(address, 0, MEM_RELEASE) }, 0);
    }
}

#[test]
fn registered_ranges_are_adjacent_half_open_and_exclude_metadata_storage() {
    let _guard = FAMILY_TEST.lock().unwrap();
    let code = TestCode::new();
    let (site, sentinel) = stub(&code, false);
    let tap = unsafe { Tap::new(site, idle, 0, sentinel) }.unwrap();
    let bridge = bridge_code(idle as *const () as usize, 0, &sentinel[..7]);
    let entries = unsafe { std::slice::from_raw_parts(table(tap.code), 4) };
    assert_eq!(
        (entries[0].BeginAddress, entries[0].EndAddress),
        (0, bridge.continuation as u32)
    );
    assert_eq!(
        (entries[1].BeginAddress, entries[1].EndAddress),
        (bridge.continuation as u32, bridge.completion as u32)
    );
    for (offset, expected) in [
        (0, 0),
        (bridge.continuation - 1, 0),
        (bridge.continuation, 1),
        (bridge.completion, 2),
        (bridge.returning - 1, 2),
        (bridge.returning, 3),
        (bridge.code.len() - 1, 3),
    ] {
        let mut base = 0;
        let found = unsafe {
            RtlLookupFunctionEntry(tap.code as u64 + offset as u64, &mut base, ptr::null_mut())
        };
        assert_eq!(base, tap.code as u64);
        assert!(!found.is_null());
        let found = unsafe { &*found };
        assert_eq!(
            (found.BeginAddress, found.EndAddress),
            (entries[expected].BeginAddress, entries[expected].EndAddress)
        );
        assert_eq!(unsafe { found.Anonymous.UnwindInfoAddress }, unsafe {
            entries[expected].Anonymous.UnwindInfoAddress
        });
    }
    for offset in [
        bridge.code.len(),
        UNWIND_OFFSET,
        LEAF_UNWIND_OFFSET,
        COMPLETION_UNWIND_OFFSET,
        TABLE_OFFSET,
        PAGE_BYTES - 1,
    ] {
        assert!(!lookup(tap.code as usize + offset));
    }
    assert!(
        tap.executable_ranges()
            .contains(&(tap.code as usize..tap.code as usize + PAGE_BYTES))
    );
}

#[test]
fn mixed_batch_preflights_all_sites_and_restores_complete_tap_spans_on_flush_failure() {
    let _guard = FAMILY_TEST.lock().unwrap();
    let code = TestCode::new();
    let (site, sentinel) = stub(&code, false);
    let mut tap = unsafe { Tap::new(site, idle, 0, sentinel) }.unwrap();
    let (ordinary_site, ordinary_bytes) = code.stub(128, idle as *const () as usize);
    let mut ordinary =
        unsafe { Hook::new(ordinary_site, idle as *const () as usize, ordinary_bytes) }.unwrap();
    unsafe {
        ptr::write((ordinary_site + 15) as *mut u8, 0xcc);
    }
    assert!(Hook::set_enabled_batch(&mut [tap.publication_hook(), &mut ordinary], true).is_err());
    assert_eq!(
        unsafe { std::slice::from_raw_parts(site as *const u8, 16) },
        sentinel
    );
    assert!(!tap.hook.is_enabled() && !ordinary.is_enabled());
    unsafe {
        ptr::write((ordinary_site + 15) as *mut u8, ordinary_bytes[15]);
    }
    fail_flushes(2);
    assert!(Hook::set_enabled_batch(&mut [tap.publication_hook(), &mut ordinary], true).is_err());
    for (address, bytes) in [(site, sentinel), (ordinary_site, ordinary_bytes)] {
        assert_eq!(
            unsafe { std::slice::from_raw_parts(address as *const u8, 16) },
            bytes
        );
    }
    assert!(!tap.hook.enabled && tap.hook.is_enabled());
    assert!(!ordinary.enabled && ordinary.is_enabled());
    assert_eq!(
        Hook::set_enabled_batch(&mut [tap.publication_hook(), &mut ordinary], false),
        Ok(true)
    );
    assert!(!tap.hook.is_enabled() && !ordinary.is_enabled());
}

#[test]
fn native_epilogue_remains_at_the_call_return_and_moved_tail_target_matches() {
    let _guard = FAMILY_TEST.lock().unwrap();
    let code = TestCode::new();
    // Synthetic push/sub/epilogue/tail target and padding, not game bytes.
    let mut bytes = [0xcc; 80];
    bytes[..5].copy_from_slice(&[0x57, 0x48, 0x83, 0xec, 0x30]);
    bytes[5..11].copy_from_slice(&[0x48, 0x83, 0xc4, 0x30, 0x5f, 0xe9]);
    bytes[11..15].copy_from_slice(&49_i32.to_le_bytes()); // JMP from +10 to +64.
    bytes[64..70].copy_from_slice(&[0xb8, 73, 0, 0, 0, 0xc3]);
    unsafe {
        ptr::copy_nonoverlapping(bytes.as_ptr(), code.0.cast(), bytes.len());
    }
    let native_entry = entry(0, 15, UNWIND_OFFSET);
    let native_table = table(code.0);
    unsafe {
        ptr::copy_nonoverlapping(
            [1_u8, 5, 2, 0, 5, 0x52, 1, 0x70].as_ptr(),
            code.0.cast::<u8>().add(UNWIND_OFFSET),
            8,
        );
        ptr::write(native_table.cast_mut(), native_entry);
    }
    assert!(unsafe { RtlAddFunctionTable(native_table, 1, code.0 as u64) });
    let _registration = NativeTable(native_table);
    let site = code.0 as usize + 5;
    let sentinel = bytes[5..21].try_into().unwrap();
    let mut tap = unsafe { Tap::new(site, idle, 0, sentinel) }.unwrap();
    fail_flushes(2);
    assert!(tap.set_enabled(true).is_err());
    assert!(!tap.hook.enabled && tap.hook.is_enabled());
    assert_eq!(
        unsafe { std::slice::from_raw_parts(site as *const u8, 16) },
        sentinel
    );
    assert!(tap.set_enabled(true).unwrap());
    let flights = tap.hook.tap.as_ref().unwrap().in_flight;
    flights.store(1, Ordering::SeqCst);
    assert!(!tap.set_enabled(false).unwrap());
    assert!(tap.hook.is_enabled());
    flights.store(0, Ordering::SeqCst);
    let patched = unsafe { std::slice::from_raw_parts(site as *const u8, 16) };
    assert_eq!(&patched[5..10], &sentinel[..5]);
    assert_eq!(
        site as i64 + 15 + i32::from_le_bytes(patched[11..15].try_into().unwrap()) as i64,
        code.0 as i64 + 64
    );
    let mut stack = [0_u64; 32];
    stack[6] = 0x1234;
    stack[7] = 0x5678;
    let native_rsp = stack.as_mut_ptr() as usize;
    let mut context = CONTEXT {
        Rip: site as u64 + 5,
        Rsp: native_rsp as u64,
        Rdi: 0xdead,
        ..Default::default()
    };
    assert!(unwind(&mut context));
    assert_eq!(
        (context.Rip, context.Rsp, context.Rdi),
        (0x5678, native_rsp as u64 + 64, 0x1234)
    );
    assert!(!lookup(site + 10)); // Moved jump has the fully restored native stack.
    let function: unsafe extern "system" fn() -> usize = unsafe { std::mem::transmute(code.0) };
    assert_eq!(unsafe { function() }, 73);
    assert!(tap.set_enabled(false).unwrap());
    assert_eq!(
        unsafe { std::slice::from_raw_parts(site as *const u8, 16) },
        sentinel
    );
}

#[test]
fn copied_load_and_add_preserve_the_native_result_and_branch_flags() {
    let _guard = FAMILY_TEST.lock().unwrap();
    let code = TestCode::new();
    let mut bytes: Vec<u8> = vec![0x53, 0x57, 0x48, 0x89, 0xcb, 0x48, 0x89, 0xd7];
    let site = code.0 as usize + bytes.len();
    // Synthetic MOV RDX,[RBX+8]; ADD RDX,[RDI], followed by a sign-dependent result.
    bytes.extend([0x48, 0x8b, 0x53, 8, 0x48, 0x03, 0x17]);
    bytes.extend([
        0x49, 0x89, 0x10, 0xb8, 0, 0, 0, 0, 0x0f, 0x98, 0xc0, 0x5f, 0x5b, 0xc3,
    ]);
    unsafe {
        ptr::copy_nonoverlapping(bytes.as_ptr(), code.0.cast(), bytes.len());
    }
    let sentinel = unsafe { std::slice::from_raw_parts(site as *const u8, 16) }
        .try_into()
        .unwrap();
    let mut tap = unsafe { Tap::new(site, idle, 0, sentinel) }.unwrap();
    assert!(lookup(tap.code as usize));
    assert!(tap.set_enabled(true).unwrap());
    let function: unsafe extern "system" fn(*const i64, *const i64, *mut i64) -> usize =
        unsafe { std::mem::transmute(code.0) };
    let adjustment = 1;
    for bank in [2_i64, i64::MAX] {
        let mut result = 0;
        let sign = unsafe { function([0, bank].as_ptr(), &adjustment, &mut result) };
        assert_eq!(result, bank.wrapping_add(adjustment));
        assert_eq!(sign, usize::from(result < 0));
    }
    assert!(tap.set_enabled(false).unwrap());
}

#[test]
fn unsupported_rsp_rip_and_branch_operands_refuse_before_installation() {
    for prefix in [
        [0x48, 0x8b, 0x44],
        [0x48, 0x8b, 0x05],
        [0x48, 0x8b, 0xa3],
        [0xe9, 0, 0],
    ] {
        let mut bytes = [0x90; 16];
        bytes[..3].copy_from_slice(&prefix);
        assert!(instruction_span(&bytes).is_err());
    }
}

#[test]
fn generic_tap_retains_the_instruction_tail_until_its_flight_returns() {
    struct Probe {
        flight: *const AtomicUsize,
        seen: usize,
    }
    unsafe extern "system-unwind" fn counted(_: &Registers, tag: usize) {
        let probe = unsafe { &mut *(tag as *mut Probe) };
        probe.seen = unsafe { &*probe.flight }.load(Ordering::SeqCst);
    }
    let _guard = FAMILY_TEST.lock().unwrap();
    let code = TestCode::new();
    let (site, sentinel) = stub(&code, false);
    let mut probe = Probe {
        flight: ptr::null(),
        seen: 0,
    };
    let tag = &mut probe as *mut Probe as usize;
    let mut tap = unsafe { Tap::new(site, counted, tag, sentinel) }.unwrap();
    let flights = tap.hook.tap.as_ref().unwrap().in_flight;
    probe.flight = flights;
    let bridge = bridge_code(counted as *const () as usize, tag, &sentinel[..7]);
    assert!(tap.set_enabled(true).unwrap());
    flights.store(1, Ordering::SeqCst);
    let patched = unsafe { std::slice::from_raw_parts(site as *const u8, 16) }.to_vec();
    assert!(!tap.set_enabled(false).unwrap());
    assert_eq!(
        unsafe { std::slice::from_raw_parts(site as *const u8, 16) },
        patched
    );
    flights.store(0, Ordering::SeqCst);
    let mut sink = [0_u64; 18];
    unsafe {
        invoke(code.0 as usize, 0x1234, 0x5678, sink.as_mut_ptr());
    }
    assert_eq!(probe.seen, 1);
    assert_eq!(flights.load(Ordering::SeqCst), 0);
    let mut stack = [0_u64; 4];
    stack[1] = site as u64 + 5;
    for (offset, rsp) in [
        (bridge.completion, stack.as_ptr() as u64 + 8),
        (bridge.completion + 1, stack.as_ptr() as u64),
        (bridge.returning - 1, stack.as_ptr() as u64),
        (bridge.returning, stack.as_ptr() as u64 + 8),
    ] {
        let mut context = CONTEXT {
            Rip: tap.code as u64 + offset as u64,
            Rsp: rsp,
            ..Default::default()
        };
        assert!(unwind(&mut context));
        assert_eq!(
            (context.Rip, context.Rsp),
            (site as u64 + 5, stack.as_ptr() as u64 + 16)
        );
    }
    assert!(tap.set_enabled(false).unwrap());
    assert_eq!(
        unsafe { std::slice::from_raw_parts(site as *const u8, 16) },
        sentinel
    );
}
