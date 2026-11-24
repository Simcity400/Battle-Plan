use super::*;

use std::sync::atomic::{AtomicUsize, Ordering};
use windows_sys::Win32::System::Diagnostics::Debug::{
    RtlCaptureContext, RtlLookupFunctionEntry, RtlVirtualUnwind,
};

static SEEN: AtomicUsize = AtomicUsize::new(0);
static FAILURES: AtomicUsize = AtomicUsize::new(0);

#[test]
fn pause_comparisons_and_condition_results_refuse_stack_dependent_forms() {
    let span = |instructions: &[u8]| {
        let mut bytes = [0x90; 16];
        bytes[..instructions.len()].copy_from_slice(instructions);
        instruction_span(&bytes)
    };
    assert_eq!(span(&[0x80, 0x79, 3, 7, 0x0f, 0x94, 0xc0]), Ok(7));
    assert_eq!(span(&[0x83, 0xfa, 17, 0x0f, 0x95, 0xc1]), Ok(6));
    assert_eq!(span(&[0xb9, 23, 0, 0, 0]), Ok(5));
    assert_eq!(span(&[0x41, 0xbc, 23, 0, 0, 0]), Ok(6));
    assert!(span(&[0xbc, 23, 0, 0, 0]).is_err());
    assert!(span(&[0x80, 0x7c, 0x24, 3, 7]).is_err());
    assert!(span(&[0x83, 0xfc, 17, 0x90, 0x90]).is_err());
    assert!(span(&[0x40, 0x0f, 0x94, 0xc4, 0x90]).is_err());
    assert!(span(&[0x0f, 0x94, 0x04, 0x24, 0x90]).is_err());
    assert!(span(&[0x80, 0x01, 7, 0x90, 0x90]).is_err());
}

#[test]
fn rip_store_relocation_preserves_target_and_immediate_and_refuses_overflow() {
    let site = 0x10000_usize;
    let mut instructions = [0xc7, 0x05, 0, 0, 0, 0, 0x12, 0x34, 0x56, 0x78];
    instructions[2..6].copy_from_slice(&(-731_i32).to_le_bytes());
    let target = rip_store_target(site, &instructions).unwrap().unwrap();
    let at = site + 0x20000;
    relocate_rip_store(&mut instructions, at, target).unwrap();
    assert_eq!(rip_store_target(at, &instructions), Ok(Some(target)));
    assert_eq!(&instructions[6..], &[0x12, 0x34, 0x56, 0x78]);
    assert!(rip_store_target(0, &[0xc7, 0x05, 0xf5, 0xff, 0xff, 0xff, 0, 0, 0, 0]).is_err());
    assert!(rip_store_target(usize::MAX, &instructions).is_err());
    assert!(relocate_rip_store(&mut instructions, usize::MAX, target).is_err());
    assert!(relocate_rip_store(&mut instructions, 0, usize::MAX).is_err());
    assert!(rip_store_target(site, &instructions[..9]).is_err());
}

#[test]
fn rip_relative_store_tap_preserves_native_value_flags_and_destination() {
    unsafe extern "system-unwind" fn observed(registers: &Registers, tag: usize) {
        assert_eq!(registers.rax, 17);
        assert_ne!(registers.flags & 1, 0);
        unsafe { (&*(tag as *const AtomicUsize)).fetch_add(1, Ordering::Relaxed) };
    }
    let _guard = FAMILY_TEST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let code = TestCode::new();
    let target = code.0 as usize + 2048;
    let mut bytes = vec![0xb8, 17, 0, 0, 0, 0xf9];
    let site = code.0 as usize + bytes.len();
    bytes.extend([0xc7, 0x05]);
    bytes.extend(
        i32::try_from(target as isize - (site + 10) as isize)
            .unwrap()
            .to_le_bytes(),
    );
    bytes.extend(0x7152_934b_u32.to_le_bytes());
    bytes.extend([0x0f, 0x92, 0xc2, 0x0f, 0xb6, 0xd2, 0x01, 0xd0, 0xc3]);
    bytes.extend([0x90; 16]);
    unsafe { ptr::copy_nonoverlapping(bytes.as_ptr(), code.0.cast(), bytes.len()) };
    let sentinel = unsafe { std::slice::from_raw_parts(site as *const u8, 16) }
        .try_into()
        .unwrap();
    let count = AtomicUsize::new(0);
    let mut tap = unsafe {
        Tap::new(
            site,
            observed,
            &raw const count as usize,
            sentinel,
        )
    }
    .unwrap();
    let invoke: unsafe extern "system" fn() -> u32 = unsafe { std::mem::transmute(code.0) };
    for enabled in [false, true, false] {
        if enabled {
            assert!(tap.set_enabled(true).unwrap());
        }
        unsafe { ptr::write(target as *mut u32, 0) };
        assert_eq!(unsafe { invoke() }, 18);
        assert_eq!(unsafe { ptr::read(target as *const u32) }, 0x7152_934b);
        if enabled {
            assert!(tap.set_enabled(false).unwrap());
        }
    }
    assert_eq!(count.load(Ordering::Relaxed), 1);
}

#[test]
fn register_xor_taps_relocate_after_restoring_native_registers_and_flags() {
    for instructions in [
        [0x49, 0x89, 0xc9, 0x31, 0xdb],
        [0x49, 0x89, 0xd9, 0x33, 0xc0],
    ] {
        let mut bytes = [0x90; 16];
        bytes[..5].copy_from_slice(&instructions);
        assert_eq!(instruction_span(&bytes), Ok(5));
        let bridge = bridge_code(1, 0, &instructions);
        assert_eq!(
            &bridge.code[bridge.continuation..bridge.completion],
            &instructions
        );
    }
    for instructions in [
        [0x48, 0x31, 0xe0, 0x90, 0x90],
        [0x48, 0x33, 0xc4, 0x90, 0x90],
        [0x31, 0x03, 0x90, 0x90, 0x90],
    ] {
        let mut bytes = [0x90; 16];
        bytes[..5].copy_from_slice(&instructions);
        assert!(instruction_span(&bytes).is_err());
    }
}

#[test]
fn faction_store_taps_accept_indexed_values_and_immediates_but_refuse_stack_operands() {
    let span = |value: &[u8]| {
        let mut bytes = [0x90; 16];
        bytes[..value.len()].copy_from_slice(value);
        instruction_span(&bytes)
    };
    assert_eq!(span(&[0x4a, 0x01, 0x4c, 0x0a, 0xf0]), Ok(5));
    assert_eq!(span(&[0x4d, 0x89, 0x4c, 0x00, 0xf0]), Ok(5));
    assert_eq!(span(&[0xc7, 0x43, 0x08, 3, 0, 0, 0]), Ok(7));
    assert_eq!(span(&[0x88, 0x88, 0x38, 0x0a, 0, 0]), Ok(6));
    assert!(span(&[0x88, 0x4c, 0x24, 8, 0x90]).is_err());
    assert_eq!(span(&[0xc6, 0x83, 0x38, 0x0a, 0, 0, 1]), Ok(7));
    assert_eq!(span(&[0x49, 0x89, 0x44, 0x24, 0x10]), Ok(5));
    assert!(span(&[0x48, 0x89, 0x44, 0x24, 0x10]).is_err());
    assert!(span(&[0xc7, 0x44, 0x24, 8, 3, 0, 0, 0]).is_err());
    assert!(span(&[0x48, 0x89, 0x24, 0x03, 0x90]).is_err());
    assert!(span(&[0x48, 0x89, 0x04, 0x25, 0, 0, 0, 0]).is_err());
    assert!(span(&[0xc7, 0x0b, 3, 0, 0, 0]).is_err());
    assert!(span(&[0xe8, 0, 0, 0, 0]).is_err());
    assert_eq!(
        span(&[0x4d, 0x0f, 0x48, 0xc7, 0x4c, 0x89, 0x40, 0x48]),
        Ok(8)
    );
    assert_eq!(span(&[0x48, 0xff, 0xc7, 0x49, 0x89, 0x48, 0x48]), Ok(7));
    assert_eq!(span(&[0x48, 0x89, 0x4d, 0x38, 0x49, 0x3b, 0xf7]), Ok(7));
    assert_eq!(span(&[0x48, 0x89, 0x4d, 0x40, 0x48, 0x83, 0xc6, 8]), Ok(8));
    assert_eq!(span(&[0x89, 0x71, 0x50, 0xff, 0xc7]), Ok(5));
    assert_eq!(span(&[0x48, 0x29, 0x88, 0x80, 0, 0, 0]), Ok(7));
    assert_eq!(
        span(&[0x48, 0x89, 0x4d, 0x40, 0x49, 0xb9, 1, 0, 0, 0, 0, 0, 0, 0]),
        Ok(14)
    );
    for bytes in [
        &[0x48, 0x0f, 0x48, 0xe0][..],
        &[0x48, 0xff, 0xc4],
        &[0x48, 0x83, 0xc4, 8],
        &[0x48, 0xbc, 0, 0, 0, 0, 0, 0, 0, 0],
    ] {
        assert!(span(bytes).is_err());
    }
}

#[test]
fn combat_log_counter_tap_compensates_for_its_return_slot_without_touching_the_counter_operand() {
    let mut code = [0x90; 16];
    code[..10].copy_from_slice(&[0xff, 0x47, 0x04, 0x8b, 0xbc, 0x24, 0x20, 0, 0, 0]);
    assert_eq!(instruction_span(&code), Ok(10));
    let bridge = bridge_code(1, 0, &code[..10]);
    assert_eq!(
        &bridge.code[bridge.continuation..bridge.completion],
        &[0xff, 0x47, 0x04, 0x8b, 0xbc, 0x24, 0x28, 0, 0, 0]
    );
    code[6..10].copy_from_slice(&i32::MAX.to_le_bytes());
    assert!(instruction_span(&code).is_err());
}

#[test]
fn lea_and_move_taps_refuse_stack_and_rip_operands() {
    for prefix in [
        [0x48, 0x8b, 0xe0],
        [0x48, 0x8b, 0xc4],
        [0x4c, 0x8b, 0xe4],
        [0x49, 0x8b, 0xe4],
    ] {
        let mut bytes = [0x90; 16];
        bytes[..3].copy_from_slice(&prefix);
        assert!(instruction_span(&bytes).is_err());
    }
    for prefix in [[0x48, 0x8d, 0x44], [0x48, 0x8d, 0x05], [0x48, 0x8d, 0xa3]] {
        let mut bytes = [0x90; 16];
        bytes[..3].copy_from_slice(&prefix);
        assert!(instruction_span(&bytes).is_err());
    }
}

fn unwind(context: &mut CONTEXT) -> bool {
    let mut aligned = AlignedContext(*context);
    let mut base = 0;
    let entry = unsafe { RtlLookupFunctionEntry(aligned.0.Rip, &raw mut base, ptr::null_mut()) };
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
            &raw mut aligned.0,
            &raw mut data,
            &raw mut frame,
            ptr::null_mut(),
        );
    }
    *context = aligned.0;
    true
}
fn lookup(address: usize) -> bool {
    let mut base = 0;
    !unsafe { RtlLookupFunctionEntry(address as u64, &raw mut base, ptr::null_mut()) }.is_null()
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
        RtlCaptureContext(&raw mut context.0);
    }
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

#[test]
fn naval_management_adjacent_taps_publish_execute_and_restore_their_owned_spans() {
    let _guard = FAMILY_TEST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let code = TestCode::new();
    let bytes = [
        0x89_u8, 0x91, 0x10, 0, 0, 0, 0x89, 0x91, 0x14, 0, 0, 0, 0xb8, 7, 0, 0, 0, 0xc3, 0x90,
        0x90, 0x90, 0x90,
    ];
    unsafe {
        ptr::copy_nonoverlapping(bytes.as_ptr(), code.0.cast(), bytes.len());
    }
    let sentinel = |offset| unsafe {
        std::slice::from_raw_parts(code.0.cast::<u8>().add(offset), 16)
            .try_into()
            .unwrap()
    };
    let mut first = unsafe { Tap::new_owned_span(code.0 as usize, idle, 0, sentinel(0)) }.unwrap();
    let mut second =
        unsafe { Tap::new_owned_span(code.0 as usize + 6, idle, 0, sentinel(6)) }.unwrap();
    let mut hooks = [first.publication_hook(), second.publication_hook()];
    assert_eq!(Hook::set_enabled_batch(&mut hooks, true), Ok(true));
    let invoke: unsafe extern "system" fn(*mut u32, u32) -> u32 =
        unsafe { std::mem::transmute(code.0) };
    let mut values = [0_u32; 8];
    assert_eq!(unsafe { invoke(values.as_mut_ptr(), 37) }, 7);
    assert_eq!((values[4], values[5]), (37, 37));
    assert_eq!(Hook::set_enabled_batch(&mut hooks, false), Ok(true));
    assert_eq!(
        unsafe { std::slice::from_raw_parts(code.0.cast::<u8>(), bytes.len()) },
        bytes
    );
}

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
        bytes.extend([0x48, 0x8d, 0x64, 0x24, 0xd8]);
    }
    let site = code.0 as usize + bytes.len();
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
    let _guard = FAMILY_TEST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
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
    let _guard = FAMILY_TEST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
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
    let _guard = FAMILY_TEST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
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
    let _guard = FAMILY_TEST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
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
            RtlLookupFunctionEntry(tap.code as u64 + offset as u64, &raw mut base, ptr::null_mut())
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
    let _guard = FAMILY_TEST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
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
    let _guard = FAMILY_TEST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let code = TestCode::new();
    let mut bytes = [0xcc; 80];
    bytes[..5].copy_from_slice(&[0x57, 0x48, 0x83, 0xec, 0x30]);
    bytes[5..11].copy_from_slice(&[0x48, 0x83, 0xc4, 0x30, 0x5f, 0xe9]);
    bytes[11..15].copy_from_slice(&49_i32.to_le_bytes());
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
    assert!(!lookup(site + 10));
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
    let _guard = FAMILY_TEST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let code = TestCode::new();
    let mut bytes: Vec<u8> = vec![0x53, 0x57, 0x48, 0x89, 0xcb, 0x48, 0x89, 0xd7];
    let site = code.0 as usize + bytes.len();
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
        let sign = unsafe { function([0, bank].as_ptr(), &raw const adjustment, &raw mut result) };
        assert_eq!(result, bank.wrapping_add(adjustment));
        assert_eq!(sign, usize::from(result < 0));
    }
    assert!(tap.set_enabled(false).unwrap());
}

#[test]
fn copied_clamp_and_store_preserve_native_flags_results_and_continuation() {
    let _guard = FAMILY_TEST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let code = TestCode::new();
    let bytes = [
        0x31, 0xc0, 0x48, 0x85, 0xc9, 0x48, 0x0f, 0x48, 0xc8, 0x48, 0x89, 0x0a, 0x48, 0x89, 0xc8,
        0xc3, 0x90, 0x90, 0x90, 0x90, 0x90,
    ];
    unsafe {
        ptr::copy_nonoverlapping(bytes.as_ptr(), code.0.cast(), bytes.len());
    }
    let site = code.0 as usize + 5;
    let sentinel = bytes[5..21].try_into().unwrap();
    let function: unsafe extern "system" fn(i64, *mut i64) -> i64 =
        unsafe { std::mem::transmute(code.0) };
    let mut tap = unsafe { Tap::new(site, idle, 0, sentinel) }.unwrap();
    for enabled in [false, true, false] {
        assert!(tap.set_enabled(enabled).unwrap());
        for value in [i64::MIN, -1, 0, 7, i64::MAX] {
            let mut stored = -99;
            assert_eq!(unsafe { function(value, &raw mut stored) }, value.max(0));
            assert_eq!(stored, value.max(0));
        }
    }
}

#[test]
fn unsupported_rsp_rip_and_branch_operands_refuse_before_installation() {
    for prefix in [
        [0x48, 0x8b, 0x44, 0x24, 8].as_slice(),
        &[0x48, 0x8b, 0x05, 0, 0, 0, 0],
        &[0x48, 0x8b, 0xa3, 0, 0, 0, 0],
        &[0xe9, 0, 0, 0, 0],
    ] {
        let mut bytes = [0x90; 16];
        bytes[..prefix.len()].copy_from_slice(prefix);
        if prefix == [0x48, 0x8b, 0x44] {
            bytes[3] = 0x24;
        }
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
    let _guard = FAMILY_TEST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let code = TestCode::new();
    let (site, sentinel) = stub(&code, false);
    let mut probe = Probe {
        flight: ptr::null(),
        seen: 0,
    };
    let tag = &raw mut probe as usize;
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

#[test]
fn doctrine_effective_gain_sib_store_preserves_operands_and_flags() {
    use std::sync::atomic::AtomicI64;
    static OPERAND: AtomicI64 = AtomicI64::new(0);
    unsafe extern "system-unwind" fn capture(registers: &Registers, _: usize) {
        OPERAND.store(registers.rax as i64, Ordering::Relaxed);
    }
    let _guard = FAMILY_TEST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let code = TestCode::new();
    let mut bytes = vec![
        0x56, 0x41, 0x56, 0x48, 0x89, 0xce, 0x4d, 0x31, 0xf6, 0x48, 0x89, 0xd0, 0x48, 0x83, 0xf8, 0,
    ];
    let site = code.0 as usize + bytes.len();
    bytes.extend([
        0x4a, 0x89, 0x44, 0x36, 0x48, 0x0f, 0x9c, 0xc0, 0x0f, 0xb6, 0xc0, 0x41, 0x5e, 0x5e, 0xc3,
        0x90,
    ]);
    unsafe {
        ptr::copy_nonoverlapping(bytes.as_ptr(), code.0.cast(), bytes.len());
    }
    let sentinel = bytes[16..32].try_into().unwrap();
    let mut tap = unsafe { Tap::new(site, capture, 0, sentinel) }.unwrap();
    assert!(lookup(tap.code as usize));
    assert!(tap.set_enabled(true).unwrap());
    let function: unsafe extern "system" fn(*mut i64, i64) -> usize =
        unsafe { std::mem::transmute(code.0) };
    for value in [-13, 0, i64::MAX] {
        let mut destination = [0_i64; 10];
        assert_eq!(
            unsafe { function(destination.as_mut_ptr(), value) },
            usize::from(value < 0)
        );
        assert_eq!(destination[9], value);
        assert_eq!(OPERAND.load(Ordering::Relaxed), value);
    }
    assert!(tap.set_enabled(false).unwrap());
}

#[test]
fn adjacent_taps_restore_owned_spans_in_serial_and_shared_publication() {
    let _guard = FAMILY_TEST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    unsafe extern "system-unwind" fn observe(_: &Registers, _: usize) {}
    let code = TestCode::new();
    let mut bytes = [0x90u8; 40];
    bytes[..2].copy_from_slice(&[0x31, 0xc0]);
    bytes[2..7].copy_from_slice(&[0x48, 0x89, 0x54, 0x01, 0x08]);
    bytes[9..14].copy_from_slice(&[0x48, 0x89, 0x54, 0x01, 0x10]);
    bytes[14] = 0xc3;
    unsafe {
        ptr::copy_nonoverlapping(bytes.as_ptr(), code.0.cast(), bytes.len());
    }
    let function: unsafe extern "system" fn(*mut u64, u64) = unsafe { std::mem::transmute(code.0) };
    for shared in [false, true] {
        let mut taps: Vec<_> = [2, 9]
            .into_iter()
            .map(|offset| {
                unsafe {
                    Tap::new(
                        code.0 as usize + offset,
                        observe,
                        offset,
                        bytes[offset..offset + 16].try_into().unwrap(),
                    )
                }
                .unwrap()
            })
            .collect();
        for enabled in [true, false] {
            let mut round = shared.then(|| super::super::shared::Round::collect().unwrap());
            if let Some(round) = &mut round {
                for tap in &mut taps {
                    assert_eq!(
                        Hook::set_family_enabled_shared(
                            std::slice::from_mut(tap.publication_hook()),
                            enabled
                        ),
                        Ok(false)
                    );
                }
                assert_eq!(round.publish().families, taps.len());
            }
            for tap in &mut taps {
                assert_eq!(
                    Hook::set_family_enabled_shared(
                        std::slice::from_mut(tap.publication_hook()),
                        enabled
                    ),
                    Ok(true)
                );
            }
            drop(round);
            let mut output = [0u64; 3];
            unsafe {
                function(output.as_mut_ptr(), 73);
            }
            assert_eq!(output, [0, 73, 73]);
            if !enabled {
                assert!(taps.iter().all(|tap| !tap.hook.is_enabled()));
                assert_eq!(
                    unsafe { std::slice::from_raw_parts(code.0.cast::<u8>(), bytes.len()) },
                    bytes
                );
            }
        }
    }
}

#[test]
fn naval_management_data_spans_reject_stack_aliases_and_control_extensions() {
    for prefix in [
        &[0x89, 0x51, 0x20, 0x49, 0x83, 0xc0, 0x10][..],
        &[0x8b, 0xd7, 0x48, 0x8d, 0x4d, 0x28],
        &[0x48, 0xc7, 0x41, 0x38, 0, 0, 0, 0],
        &[0x49, 0x8b, 0x57, 0x28, 0x48, 0x63, 0xc6],
        &[0x48, 0x03, 0xd0, 0x48, 0x29, 0x51, 0x38],
        &[0x40, 0x88, 0xb1, 0xf7, 4, 0, 0],
    ] {
        let mut bytes = [0x90; 16];
        bytes[..prefix.len()].copy_from_slice(prefix);
        assert_eq!(instruction_span(&bytes), Ok(prefix.len()));
    }
    for prefix in [
        &[0x48, 0x8b, 0xe0][..],
        &[0x48, 0x8b, 0xc4],
        &[0x48, 0x83, 0xc4, 8],
        &[0x48, 0xff, 0x51, 8],
        &[0x48, 0xff, 0x31],
        &[0x48, 0x8d, 0x0d, 0, 0, 0, 0],
        &[0x48, 0x63, 0x44, 0x24, 0x20],
    ] {
        let mut bytes = [0x90; 16];
        bytes[..prefix.len()].copy_from_slice(prefix);
        assert!(instruction_span(&bytes).is_err(), "{prefix:x?}");
    }
}
#[test]
fn naval_management_store_and_immediate_span_executes_with_original_operands() {
    let _guard = FAMILY_TEST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let code = TestCode::new();
    let mut bytes = [0x90; 32];
    bytes[..11].copy_from_slice(&[
        0x89, 0x51, 0x20, 0x49, 0x83, 0xc0, 0x10, 0x8b, 0x41, 0x20, 0xc3,
    ]);
    unsafe {
        ptr::copy_nonoverlapping(bytes.as_ptr(), code.0.cast(), bytes.len());
    }
    let mut tap =
        unsafe { Tap::new(code.0 as usize, idle, 0, bytes[..16].try_into().unwrap()) }.unwrap();
    assert!(tap.set_enabled(true).unwrap());
    let call: unsafe extern "system" fn(*mut [u32; 16], u32, usize) -> u32 =
        unsafe { std::mem::transmute(code.0) };
    for value in [0, u32::MAX, 37] {
        let mut object = [0; 16];
        assert_eq!(unsafe { call(&raw mut object, value, 9) }, value);
        assert_eq!(object[8], value);
    }
    assert!(tap.set_enabled(false).unwrap());
}

#[test]
fn script_writer_copied_lea_register_moves_zero_and_decrement_execute_with_native_flags() {
    let _guard = FAMILY_TEST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for mode in 0..5 {
        let code = TestCode::new();
        let mut bytes: Vec<u8> = vec![
            0x53, 0x56, 0x57, 0x41, 0x56, 0x48, 0x89, 0xcb, 0x48, 0x89, 0xd7,
        ];
        if mode == 2 {
            bytes.extend([0x48, 0x89, 0xce]);
        }
        bytes.push(0xf9);
        let site = code.0 as usize + bytes.len();
        match mode {
            0 => bytes.extend([0x48, 0x8d, 0x8b, 0x18, 0x01, 0, 0]),
            1 => bytes.extend([0x48, 0x8b, 0xf3, 0x45, 0x33, 0xf6]),
            2 => bytes.extend([0xff, 0x8e, 0x38, 0x05, 0, 0]),
            3 => bytes.extend([0xc7, 0x43, 0x28, 0, 0, 0, 0]),
            _ => bytes.extend([0x48, 0x8b, 0xcb, 0xba, 1, 0, 0, 0]),
        }
        bytes.extend([0x9c, 0x58, 0x48, 0x89, 0x07]);
        match mode {
            0 => bytes.extend([0x48, 0x89, 0xc8]),
            1 => bytes.extend([0x4c, 0x89, 0xf0]),
            2 => bytes.extend([0x8b, 0x86, 0x38, 0x05, 0, 0]),
            3 => bytes.extend([0x8b, 0x43, 0x28]),
            _ => bytes.extend([0x48, 0x89, 0xd0]),
        }
        bytes.extend([0x41, 0x5e, 0x5f, 0x5e, 0x5b, 0xc3]);
        unsafe {
            ptr::copy_nonoverlapping(bytes.as_ptr(), code.0.cast(), bytes.len());
        }
        let sentinel = unsafe { std::slice::from_raw_parts(site as *const u8, 16) }
            .try_into()
            .unwrap();
        let mut tap = unsafe { Tap::new(site, idle, 0, sentinel) }.unwrap();
        assert!(tap.set_enabled(true).unwrap());
        let function: unsafe extern "system" fn(*mut u32, *mut usize) -> usize =
            unsafe { std::mem::transmute(code.0) };
        let mut object = [0_u32; 0x540 / 4];
        object[0x538 / 4] = 0;
        object[0x28 / 4] = 7;
        let mut flags = 0;
        let result = unsafe { function(object.as_mut_ptr(), &raw mut flags) };
        assert_eq!(flags & 1, usize::from(mode != 1));
        match mode {
            0 => assert_eq!(result, object.as_ptr() as usize + 0x118),
            1 => {
                assert_eq!(result, 0);
                assert_ne!(flags & 0x40, 0);
            }
            3 => {
                assert_eq!(result, 0);
                assert_eq!(object[0x28 / 4], 0);
            }
            4 => assert_eq!(result, 1),
            _ => {
                assert_eq!(result, u32::MAX as usize);
                assert_eq!(object[0x538 / 4], u32::MAX);
                assert_ne!(flags & 0x80, 0);
            }
        }
        assert!(tap.set_enabled(false).unwrap());
    }
}

#[test]
fn script_lifecycle_store_and_jump_preserve_execution_flags_and_native_unwind_continuation() {
    struct Probe {
        target: usize,
        seen: usize,
    }
    unsafe extern "system-unwind" fn observed(registers: &Registers, tag: usize) {
        let probe = unsafe { &mut *(tag as *mut Probe) };
        assert_eq!(
            unsafe { ptr::read((registers.rsp() - 8) as *const usize) },
            probe.target
        );
        assert_eq!(registers.flags & 1, 1);
        let mut context = AlignedContext(unsafe { std::mem::zeroed() });
        unsafe {
            RtlCaptureContext(&raw mut context.0);
        }
        for _ in 0..4 {
            if context.0.Rip == probe.target as u64 {
                break;
            }
            assert!(unwind(&mut context.0));
        }
        assert_eq!(context.0.Rip, probe.target as u64);
        assert_eq!(context.0.Rsp, registers.rsp() as u64);
        probe.seen += 1;
    }
    let _guard = FAMILY_TEST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let code = TestCode::new();
    let mut bytes: Vec<u8> = vec![
        0x53, 0x56, 0x41, 0x54, 0x48, 0x89, 0xcb, 0x48, 0x89, 0xd6, 0x45, 0x33, 0xe4, 0xf9,
    ];
    let site = code.0 as usize + bytes.len();
    bytes.extend([0x44, 0x89, 0x63, 0x28, 0xe9, 5, 0, 0, 0, 0xb8, 13, 0, 0, 0]);
    let target = code.0 as usize + bytes.len();
    bytes.extend([0x8b, 0x43, 0x28, 0x89, 0x06, 0x41, 0x5c, 0x5e, 0x5b, 0xc3]);
    unsafe {
        ptr::copy_nonoverlapping(bytes.as_ptr(), code.0.cast(), bytes.len());
    }
    let sentinel = unsafe { std::slice::from_raw_parts(site as *const u8, 16) }
        .try_into()
        .unwrap();
    let mut probe = Probe { target, seen: 0 };
    let mut tap =
        unsafe { Tap::new(site, observed, &raw mut probe as usize, sentinel) }.unwrap();
    assert!(tap.set_enabled(true).unwrap());
    let function: unsafe extern "system" fn(*mut u32, *mut u32) -> u32 =
        unsafe { std::mem::transmute(code.0) };
    let mut object = [0_u32; 16];
    object[10] = 7;
    let mut output = 99;
    assert_eq!(unsafe { function(object.as_mut_ptr(), &raw mut output) }, 0);
    assert_eq!((object[10], output, probe.seen), (0, 0, 1));
    assert!(tap.set_enabled(false).unwrap());
    object[10] = 11;
    assert_eq!(unsafe { function(object.as_mut_ptr(), &raw mut output) }, 0);
    assert_eq!((object[10], probe.seen), (0, 1));
}

#[test]
fn post_effect_reactivation_bypasses_the_terminal_store_observer() {
    unsafe extern "system-unwind" fn observed(_: &Registers, tag: usize) {
        let count = unsafe { &*(tag as *const AtomicUsize) };
        count.fetch_add(1, Ordering::Relaxed);
    }
    let _guard = FAMILY_TEST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let code = TestCode::new();
    let mut bytes: Vec<u8> = vec![
        0x53, 0x48, 0x89, 0xcb, 0x89, 0x53, 0x1c, 0x83, 0x7b, 0x1c, 0, 0x74, 7,
    ];
    let site = code.0 as usize + bytes.len();
    bytes.extend([0xc7, 0x43, 0x1c, 4, 0, 0, 0]);
    bytes.extend([0x8b, 0x43, 0x1c, 0x5b, 0xc3]);
    bytes.extend([0x90; 16]);
    unsafe {
        ptr::copy_nonoverlapping(bytes.as_ptr(), code.0.cast(), bytes.len());
    }
    let sentinel = unsafe { std::slice::from_raw_parts(site as *const u8, 16) }
        .try_into()
        .unwrap();
    let count = AtomicUsize::new(0);
    let mut tap = unsafe {
        Tap::new(
            site,
            observed,
            &raw const count as usize,
            sentinel,
        )
    }
    .unwrap();
    assert!(tap.set_enabled(true).unwrap());
    let invoke: unsafe extern "system" fn(*mut u32, u32) -> u32 =
        unsafe { std::mem::transmute(code.0) };
    let mut instance = [0_u32; 16];
    assert_eq!(unsafe { invoke(instance.as_mut_ptr(), 0) }, 0);
    assert_eq!(count.load(Ordering::Relaxed), 0);
    assert_eq!(unsafe { invoke(instance.as_mut_ptr(), 1) }, 4);
    assert_eq!(count.load(Ordering::Relaxed), 1);
    assert!(tap.set_enabled(false).unwrap());
}

#[test]
fn targeted_duration_store_jump_executes_once_and_preserves_native_continuation() {
    unsafe extern "system-unwind" fn observed(registers: &Registers, tag: usize) {
        assert_eq!(registers.rax as u32, 17);
        unsafe {
            (&*(tag as *const AtomicUsize)).fetch_add(1, Ordering::Relaxed);
        }
    }
    let _guard = FAMILY_TEST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let code = TestCode::new();
    let mut bytes: Vec<u8> = vec![0x53, 0x48, 0x89, 0xcb, 0xb8, 17, 0, 0, 0];
    let site = code.0 as usize + bytes.len();
    bytes.extend([0x89, 0x43, 0x30, 0xe9, 5, 0, 0, 0, 0xb8, 99, 0, 0, 0]);
    bytes.extend([0x8b, 0x43, 0x30, 0x5b, 0xc3]);
    bytes.extend([0x90; 16]);
    unsafe {
        ptr::copy_nonoverlapping(bytes.as_ptr(), code.0.cast(), bytes.len());
    }
    let sentinel = unsafe { std::slice::from_raw_parts(site as *const u8, 16) }
        .try_into()
        .unwrap();
    let count = AtomicUsize::new(0);
    let mut tap = unsafe {
        Tap::new(
            site,
            observed,
            &raw const count as usize,
            sentinel,
        )
    }
    .unwrap();
    assert!(tap.set_enabled(true).unwrap());
    let invoke: unsafe extern "system" fn(*mut u32) -> u32 = unsafe { std::mem::transmute(code.0) };
    let mut instance = [0_u32; 16];
    assert_eq!(unsafe { invoke(instance.as_mut_ptr()) }, 17);
    assert_eq!(instance[0x30 / 4], 17);
    assert_eq!(count.load(Ordering::Relaxed), 1);
    assert!(tap.set_enabled(false).unwrap());
}

#[test]
fn taken_membership_writer_is_observed_with_empty_and_nonempty_remove_effects() {
    unsafe extern "system-unwind" fn observed(registers: &Registers, tag: usize) {
        assert_eq!(
            unsafe { ptr::read((registers.rsi + 0x4c) as *const u32) },
            0
        );
        assert_ne!(registers.rdi, 0);
        unsafe {
            (&*(tag as *const AtomicUsize)).fetch_add(1, Ordering::Relaxed);
        }
    }
    let _guard = FAMILY_TEST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let code = TestCode::new();
    let mut bytes: Vec<u8> = vec![
        0x53, 0x56, 0x57, 0x48, 0x89, 0xce, 0x48, 0x89, 0xd7, 0xff, 0x4e, 0x4c,
    ];
    let site = code.0 as usize + bytes.len();
    bytes.extend([0x48, 0x8b, 0x56, 8, 0x48, 0x83, 0xc2, 8]);
    bytes.extend([
        0x45, 0x85, 0xc0, 0x74, 2, 0x90, 0x90, 0x8b, 0x46, 0x4c, 0x5f, 0x5e, 0x5b, 0xc3,
    ]);
    bytes.extend([0x90; 16]);
    unsafe {
        ptr::copy_nonoverlapping(bytes.as_ptr(), code.0.cast(), bytes.len());
    }
    let sentinel = unsafe { std::slice::from_raw_parts(site as *const u8, 16) }
        .try_into()
        .unwrap();
    let count = AtomicUsize::new(0);
    let mut tap = unsafe {
        Tap::new(
            site,
            observed,
            &raw const count as usize,
            sentinel,
        )
    }
    .unwrap();
    assert!(tap.set_enabled(true).unwrap());
    let invoke: unsafe extern "system" fn(*mut u64, usize, u32) -> u32 =
        unsafe { std::mem::transmute(code.0) };
    let mut status = [0_u64; 16];
    for effects in [0, 1] {
        unsafe {
            ptr::write((status.as_mut_ptr() as usize + 0x4c) as *mut u32, 1);
        }
        assert_eq!(unsafe { invoke(status.as_mut_ptr(), 99, effects) }, 0);
    }
    assert_eq!(count.load(Ordering::Relaxed), 2);
    assert!(tap.set_enabled(false).unwrap());
}

#[test]
fn duration_add_operands_survive_the_tap_and_reach_the_selected_scalar_writer() {
    struct Probe {
        cooldown: bool,
        seen: usize,
    }
    unsafe extern "system-unwind" fn observed(registers: &Registers, tag: usize) {
        let probe = unsafe { &mut *(tag as *mut Probe) };
        let (object, delta, offset) = if probe.cooldown {
            (
                registers.rax,
                unsafe { ptr::read((registers.rbx + 0x78) as *const i32) },
                0x10,
            )
        } else {
            (registers.rdi, registers.rax as i32, 0x18)
        };
        assert_eq!(unsafe { ptr::read((object + offset) as *const i32) }, 10);
        assert_eq!(delta, 5);
        assert_ne!(registers.flags & 1, 0);
        probe.seen += 1;
    }
    let _guard = FAMILY_TEST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for cooldown in [false, true] {
        let code = TestCode::new();
        let mut bytes: Vec<u8> = if cooldown {
            vec![0x53, 0x48, 0x8b, 0xda, 0x48, 0x8b, 0xc1, 0xf9]
        } else {
            vec![0x57, 0x48, 0x8b, 0xf9, 0x8b, 0xc2, 0xf9]
        };
        let site = code.0 as usize + bytes.len();
        if cooldown {
            bytes.extend([
                0x8b, 0x53, 0x78, 0x48, 0x8b, 0xc8, 0x01, 0x51, 0x10, 0x8b, 0x41, 0x10, 0x5b, 0xc3,
            ]);
        } else {
            bytes.extend([
                0x8b, 0xd0, 0x48, 0x8b, 0xcf, 0x01, 0x51, 0x18, 0x8b, 0x41, 0x18, 0x5f, 0xc3,
            ]);
        }
        bytes.extend([0x90; 16]);
        unsafe {
            ptr::copy_nonoverlapping(bytes.as_ptr(), code.0.cast(), bytes.len());
        }
        let sentinel = unsafe { std::slice::from_raw_parts(site as *const u8, 16) }
            .try_into()
            .unwrap();
        let mut probe = Probe { cooldown, seen: 0 };
        let mut tap =
            unsafe { Tap::new(site, observed, &raw mut probe as usize, sentinel) }
                .unwrap();
        assert!(tap.set_enabled(true).unwrap());
        let mut object = [0_i32; 32];
        let offset = if cooldown { 0x10 / 4 } else { 0x18 / 4 };
        object[offset] = 10;
        let mut effect = [0_i32; 32];
        effect[0x78 / 4] = 5;
        let invoke: unsafe extern "system" fn(*mut i32, usize) -> i32 =
            unsafe { std::mem::transmute(code.0) };
        let operand = if cooldown {
            effect.as_ptr() as usize
        } else {
            5
        };
        assert_eq!(unsafe { invoke(object.as_mut_ptr(), operand) }, 15);
        assert_eq!((object[offset], probe.seen), (15, 1));
        assert!(tap.set_enabled(false).unwrap());
    }
}

#[test]
fn selection_default_moves_preserve_operands_flags_and_final_native_duration() {
    unsafe extern "system-unwind" fn observed(registers: &Registers, tag: usize) {
        assert_ne!(registers.flags & 1, 0);
        assert!(matches!(registers.rax as i32, 0 | 17));
        unsafe {
            (&*(tag as *const AtomicUsize)).fetch_add(1, Ordering::Relaxed);
        }
    }
    let _guard = FAMILY_TEST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for targeted in [false, true] {
        let code = TestCode::new();
        let mut bytes: Vec<u8> = if targeted {
            vec![0x41, 0x57, 0x41, 0xbf, 1, 0, 0, 0, 0x8b, 0xc1, 0xf9]
        } else {
            vec![0x53, 0xbb, 1, 0, 0, 0, 0x8b, 0xc1, 0xf9]
        };
        let site = code.0 as usize + bytes.len();
        if targeted {
            bytes.extend([
                0x85, 0xc0, 0x41, 0x0f, 0x44, 0xc7, 0x89, 0x02, 0x41, 0x5f, 0xc3,
            ]);
        } else {
            bytes.extend([0x85, 0xc0, 0x0f, 0x44, 0xc3, 0x89, 0x02, 0x5b, 0xc3]);
        }
        bytes.extend([0x90; 16]);
        unsafe {
            ptr::copy_nonoverlapping(bytes.as_ptr(), code.0.cast(), bytes.len());
        }
        let sentinel = unsafe { std::slice::from_raw_parts(site as *const u8, 16) }
            .try_into()
            .unwrap();
        let count = AtomicUsize::new(0);
        let mut tap = unsafe {
            Tap::new(
                site,
                observed,
                &raw const count as usize,
                sentinel,
            )
        }
        .unwrap();
        assert!(tap.set_enabled(true).unwrap());
        let invoke: unsafe extern "system" fn(i32, *mut i32) -> i32 =
            unsafe { std::mem::transmute(code.0) };
        for (operand, expected) in [(0, 1), (17, 17)] {
            let mut stored = 0;
            assert_eq!(unsafe { invoke(operand, &raw mut stored) }, expected);
            assert_eq!(stored, expected);
        }
        assert_eq!(count.load(Ordering::Relaxed), 2);
        assert!(tap.set_enabled(false).unwrap());
    }
}
