#[cfg(not(all(windows, target_arch = "x86_64", target_env = "msvc")))]
compile_error!("this oracle requires Windows x64 MSVC");

use collector_hooks::hook;

use hook::{Registers, Tap};
use std::{
    ffi::c_void,
    mem::{offset_of, size_of},
    ptr,
};
use windows_sys::Win32::System::{
    Diagnostics::Debug::{
        FlushInstructionCache, IMAGE_RUNTIME_FUNCTION_ENTRY, IMAGE_RUNTIME_FUNCTION_ENTRY_0,
        RtlAddFunctionTable, RtlDeleteFunctionTable, RtlLookupFunctionEntry,
    },
    Memory::{
        MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_EXECUTE_READ, PAGE_READWRITE, VirtualAlloc,
        VirtualFree, VirtualProtect,
    },
    Threading::GetCurrentProcess,
};

#[path = "../../../tools/tap-foreign-unwind/runtime.rs"]
mod native_probe;
use native_probe::{Entry, Raise};

#[repr(C)]
#[derive(Default)]
struct Probe {
    entered: usize,
    exited: usize,
    active: usize,
    dropped: usize,
    continued: usize,
    expected_rsp: usize,
    seen_rsp: usize,
    expected_return: usize,
    failures: usize,
    raise: Option<Raise>,
}
const NATIVE_REGISTERS: [usize; 8] = [
    0x1101, 0x1102, 0x1103, 0x1104, 0x1105, 0x1106, 0x1107, 0x1108,
];
struct Active(*mut Probe);
impl Drop for Active {
    fn drop(&mut self) {
        unsafe {
            (*self.0).active -= 1;
            (*self.0).dropped += 1;
        }
    }
}

#[inline(never)]
unsafe extern "system-unwind" fn observe(registers: &Registers, tag: usize) {
    let probe = tag as *mut Probe;
    unsafe {
        (*probe).entered += 1;
        (*probe).active += 1;
    }
    let _active = Active(probe);
    let native_rsp = registers as *const Registers as usize + 208;
    unsafe {
        (*probe).seen_rsp = native_rsp;
        if registers.rcx != tag
            || native_rsp != (*probe).expected_rsp
            || ptr::read((native_rsp - 8) as *const usize) != (*probe).expected_return
            || [
                registers.rbx,
                registers.rbp,
                registers.rsi,
                registers.rdi,
                registers.r12,
                registers.r13,
                registers.r14,
                registers.r15,
            ] != NATIVE_REGISTERS
        {
            (*probe).failures |= 1;
        }
        if let Some(raise) = (*probe).raise {
            raise();
        }
        (*probe).exited += 1;
    }
}

struct Native {
    page: *mut c_void,
    table: *const IMAGE_RUNTIME_FUNCTION_ENTRY,
    site: usize,
    sentinel: [u8; 16],
}
impl Native {
    fn new() -> Self {
        const PAGE: usize = 4096;
        const UNWIND: usize = 512;
        const TABLE: usize = 560;
        let page =
            unsafe { VirtualAlloc(ptr::null(), PAGE, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE) };
        assert!(!page.is_null());
        let mut code = Vec::new();
        let mut operations = Vec::new();
        for register in [3_u8, 5, 6, 7, 12, 13, 14, 15] {
            if register >= 8 {
                code.push(0x41);
            }
            code.push(0x50 | (register & 7));
            operations.push([code.len() as u8, register << 4]);
        }
        code.extend([0x48, 0x83, 0xec, 0x28]);
        operations.push([code.len() as u8, 0x42]);
        let mut unwind = vec![1, code.len() as u8, operations.len() as u8, 0];
        for operation in operations.into_iter().rev() {
            unwind.extend(operation);
        }
        while unwind.len() % 4 != 0 {
            unwind.push(0);
        }
        for (register, value) in [3_u8, 5, 6, 7, 12, 13, 14, 15]
            .into_iter()
            .zip(NATIVE_REGISTERS)
        {
            code.extend([
                if register >= 8 { 0x49 } else { 0x48 },
                0xb8 | (register & 7),
            ]);
            code.extend(value.to_le_bytes());
        }
        code.extend([0x48, 0x89, 0x61, offset_of!(Probe, expected_rsp) as u8]);
        let site = page as usize + code.len();
        code.extend([0x48, 0x8b, 0x41, 0x08, 0x48, 0x8b, 0x11]);
        code.extend([0x48, 0xff, 0x41, offset_of!(Probe, continued) as u8]);
        code.extend([0x48, 0x83, 0xc4, 0x28]);
        for register in [15_u8, 14, 13, 12, 7, 6, 5, 3] {
            if register >= 8 {
                code.push(0x41);
            }
            code.push(0x58 | (register & 7));
        }
        code.push(0xc3);
        assert!(code.len() <= UNWIND && UNWIND + unwind.len() <= TABLE);
        let table = unsafe {
            page.cast::<u8>()
                .add(TABLE)
                .cast::<IMAGE_RUNTIME_FUNCTION_ENTRY>()
        };
        let entry = IMAGE_RUNTIME_FUNCTION_ENTRY {
            BeginAddress: 0,
            EndAddress: code.len() as u32,
            Anonymous: IMAGE_RUNTIME_FUNCTION_ENTRY_0 {
                UnwindInfoAddress: UNWIND as u32,
            },
        };
        unsafe {
            ptr::copy_nonoverlapping(code.as_ptr(), page.cast(), code.len());
            ptr::copy_nonoverlapping(unwind.as_ptr(), page.cast::<u8>().add(UNWIND), unwind.len());
            ptr::write(table, entry);
            let mut previous = 0;
            assert_ne!(
                VirtualProtect(page, PAGE, PAGE_EXECUTE_READ, &mut previous),
                0
            );
            assert_ne!(
                FlushInstructionCache(GetCurrentProcess(), page, code.len()),
                0
            );
            assert!(RtlAddFunctionTable(table, 1, page as u64));
        }
        let sentinel = unsafe { std::slice::from_raw_parts(site as *const u8, 16) }
            .try_into()
            .unwrap();
        Self {
            page,
            table,
            site,
            sentinel,
        }
    }
}
impl Drop for Native {
    fn drop(&mut self) {
        assert!(unsafe { RtlDeleteFunctionTable(self.table) });
        assert_ne!(unsafe { VirtualFree(self.page, 0, MEM_RELEASE) }, 0);
    }
}
fn lookup(address: usize) -> bool {
    let mut base = 0;
    !unsafe { RtlLookupFunctionEntry(address as u64, &mut base, ptr::null_mut()) }.is_null()
}

fn main() {
    let _probe_guard = native_probe::SERIAL.lock().unwrap();
    assert_eq!(size_of::<Registers>(), 128);
    assert!(offset_of!(Probe, expected_rsp) < 128 && offset_of!(Probe, continued) < 128);
    let native = Native::new();
    let mut probe = Probe::default();
    let probe_pointer = ptr::addr_of_mut!(probe);
    let mut tap = unsafe {
        Tap::new(
            native.site,
            observe,
            probe_pointer as usize,
            native.sentinel,
        )
    }
    .unwrap();
    let bridge = tap.executable_ranges().last().unwrap().start;
    assert!(lookup(bridge));
    println!(
        "oracle native={:#x} site={:#x} bridge={bridge:#x}",
        native.page as usize, native.site
    );
    assert!(hook::Hook::set_enabled_batch(&mut [tap.publication_hook()], true).unwrap());
    let drivers = native_probe::Probe::load();
    let entry: Entry = unsafe { std::mem::transmute(native.page) };
    for case in drivers.cases() {
        let name = case.name;
        let raise = case.raise;
        unsafe {
            ptr::write(
                probe_pointer,
                Probe {
                    expected_return: native.site + 5,
                    raise,
                    ..Default::default()
                },
            );
        }
        unsafe { drivers.run(case, entry, probe_pointer as usize) };
        assert_eq!(
            (probe.entered, probe.active, probe.dropped, probe.failures),
            (1, 0, 1, 0),
            "{name} Rust cleanup and generated caller"
        );
        assert_eq!(
            probe.seen_rsp, probe.expected_rsp,
            "{name} native RSP at the interior site"
        );
        let normal = usize::from(raise.is_none());
        assert_eq!(
            (probe.exited, probe.continued),
            (normal, normal),
            "{name} native instruction continuation"
        );
        println!(
            "PASS {name}: native handler, Rust cleanup, generated caller, nonvolatiles, RSP, continuation"
        );
    }
    assert!(!hook::Hook::set_enabled_batch(&mut [tap.publication_hook()], false).unwrap());
    assert_eq!(unsafe { *(native.site as *const u8) }, 0xe8);
    drop(tap);
    assert!(lookup(bridge));
    std::mem::forget(native);
    println!("PASS retention: refused restoration and retained generated unwind lookup");
}
