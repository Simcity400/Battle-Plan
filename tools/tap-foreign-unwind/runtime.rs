//! Test-only client of tools/tap-foreign-unwind/probe.cpp, never a Rust catcher.
use std::{os::windows::ffi::OsStrExt, sync::Mutex};
use windows_sys::Win32::{
    Foundation::{FreeLibrary, HMODULE},
    System::LibraryLoader::{GetProcAddress, LoadLibraryW},
};

pub static SERIAL: Mutex<()> = Mutex::new(());
pub type Entry = unsafe extern "system-unwind" fn(usize);
pub type Raise = unsafe extern "system-unwind" fn();
pub type Driver = unsafe extern "system" fn(Entry, usize) -> u32;

pub struct Probe {
    module: HMODULE,
    cpp: Driver,
    seh: Driver,
    raise_cpp: Raise,
    raise_seh: Raise,
}
#[derive(Clone, Copy)]
pub struct Case {
    pub name: &'static str,
    pub expected: u32,
    driver: Driver,
    pub raise: Option<Raise>,
}
impl Probe {
    pub fn load() -> Self {
        let path = std::env::var_os("BATTLE_PLAN_TAP_UNWIND_PROBE").or_else(|| std::env::var_os("HOI4_TAP_UNWIND_PROBE"))
            .unwrap_or_else(|| "tap_foreign_probe.dll".into());
        let path: Vec<_> = path.encode_wide().chain(Some(0)).collect();
        let module = unsafe { LoadLibraryW(path.as_ptr()) };
        assert!(!module.is_null(), "load shared native probe DLL");
        // Address conversion assigns each native export its documented ABI;
        // no non-unwinding Rust callback is cast into an unwind-capable one.
        unsafe {
            Self {
                module,
                cpp: std::mem::transmute(
                    GetProcAddress(module, c"tap_probe_cpp".as_ptr().cast()).unwrap(),
                ),
                seh: std::mem::transmute(
                    GetProcAddress(module, c"tap_probe_seh".as_ptr().cast()).unwrap(),
                ),
                raise_cpp: std::mem::transmute(
                    GetProcAddress(module, c"tap_probe_raise_cpp".as_ptr().cast()).unwrap(),
                ),
                raise_seh: std::mem::transmute(
                    GetProcAddress(module, c"tap_probe_raise_seh".as_ptr().cast()).unwrap(),
                ),
            }
        }
    }
    pub fn cases(&self) -> [Case; 3] {
        [
            Case {
                name: "normal",
                expected: 1,
                driver: self.cpp,
                raise: None,
            },
            Case {
                name: "C++",
                expected: 2,
                driver: self.cpp,
                raise: Some(self.raise_cpp),
            },
            Case {
                name: "SEH",
                expected: 3,
                driver: self.seh,
                raise: Some(self.raise_seh),
            },
        ]
    }
    /// # Safety
    /// Entry and its argument must remain valid through normal and foreign
    /// dispatch; the shared native handlers must catch before returning here.
    pub unsafe fn run(&self, case: Case, entry: Entry, argument: usize) {
        let mut witness = [0usize; 30];
        // Both native drivers catch before their non-unwinding return. The
        // shared authored wrapper records the actual native handler continuation.
        let result = unsafe { invoke(case.driver, entry, argument, witness.as_mut_ptr()) };
        assert_eq!(
            result, case.expected,
            "{} native handler/cleanup",
            case.name
        );
        assert_eq!(
            &witness[..8],
            &WRAPPER_REGISTERS,
            "{} native nonvolatile continuation",
            case.name
        );
        assert_eq!(
            witness[8], witness[9],
            "{} actual RSP continuation",
            case.name
        );
        for (index, xmm) in witness[10..].chunks_exact(2).enumerate() {
            assert_eq!(
                xmm,
                &[0x3301 + index, 0],
                "{} XMM{} continuation",
                case.name,
                index + 6
            );
        }
    }
}
impl Drop for Probe {
    fn drop(&mut self) {
        assert_ne!(unsafe { FreeLibrary(self.module) }, 0);
    }
}
const WRAPPER_REGISTERS: [usize; 8] = [
    0x2201, 0x2202, 0x2203, 0x2204, 0x2205, 0x2206, 0x2207, 0x2208,
];

// The native DLL catches all exceptions before this wrapper returns. Set every
// nonvolatile, then witness its actual return state before restoring our caller.
#[unsafe(naked)]
unsafe extern "system" fn invoke(
    _driver: Driver,
    _entry: Entry,
    _argument: usize,
    _witness: *mut usize,
) -> u32 {
    core::arch::naked_asm!(
        "push rbx",
        "push rbp",
        "push rsi",
        "push rdi",
        "push r12",
        "push r13",
        "push r14",
        "push r15",
        "sub rsp, 216",
        "mov [rsp + 32], r9",
        "mov [rsp + 40], rsp",
        "movdqu [rsp + 48], xmm6",
        "movdqu [rsp + 64], xmm7",
        "movdqu [rsp + 80], xmm8",
        "movdqu [rsp + 96], xmm9",
        "movdqu [rsp + 112], xmm10",
        "movdqu [rsp + 128], xmm11",
        "movdqu [rsp + 144], xmm12",
        "movdqu [rsp + 160], xmm13",
        "movdqu [rsp + 176], xmm14",
        "movdqu [rsp + 192], xmm15",
        "mov r11, rcx",
        "mov rcx, rdx",
        "mov rdx, r8",
        "mov rbx, 0x2201",
        "mov rbp, 0x2202",
        "mov rsi, 0x2203",
        "mov rdi, 0x2204",
        "mov r12, 0x2205",
        "mov r13, 0x2206",
        "mov r14, 0x2207",
        "mov r15, 0x2208",
        "mov eax, 0x3301",
        "movq xmm6, rax",
        "mov eax, 0x3302",
        "movq xmm7, rax",
        "mov eax, 0x3303",
        "movq xmm8, rax",
        "mov eax, 0x3304",
        "movq xmm9, rax",
        "mov eax, 0x3305",
        "movq xmm10, rax",
        "mov eax, 0x3306",
        "movq xmm11, rax",
        "mov eax, 0x3307",
        "movq xmm12, rax",
        "mov eax, 0x3308",
        "movq xmm13, rax",
        "mov eax, 0x3309",
        "movq xmm14, rax",
        "mov eax, 0x330a",
        "movq xmm15, rax",
        "call r11",
        "mov r10, [rsp + 32]",
        "mov [r10], rbx",
        "mov [r10 + 8], rbp",
        "mov [r10 + 16], rsi",
        "mov [r10 + 24], rdi",
        "mov [r10 + 32], r12",
        "mov [r10 + 40], r13",
        "mov [r10 + 48], r14",
        "mov [r10 + 56], r15",
        "mov [r10 + 64], rsp",
        "mov r11, [rsp + 40]",
        "mov [r10 + 72], r11",
        "movdqu [r10 + 80], xmm6",
        "movdqu [r10 + 96], xmm7",
        "movdqu [r10 + 112], xmm8",
        "movdqu [r10 + 128], xmm9",
        "movdqu [r10 + 144], xmm10",
        "movdqu [r10 + 160], xmm11",
        "movdqu [r10 + 176], xmm12",
        "movdqu [r10 + 192], xmm13",
        "movdqu [r10 + 208], xmm14",
        "movdqu [r10 + 224], xmm15",
        "movdqu xmm6, [rsp + 48]",
        "movdqu xmm7, [rsp + 64]",
        "movdqu xmm8, [rsp + 80]",
        "movdqu xmm9, [rsp + 96]",
        "movdqu xmm10, [rsp + 112]",
        "movdqu xmm11, [rsp + 128]",
        "movdqu xmm12, [rsp + 144]",
        "movdqu xmm13, [rsp + 160]",
        "movdqu xmm14, [rsp + 176]",
        "movdqu xmm15, [rsp + 192]",
        "add rsp, 216",
        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop rdi",
        "pop rsi",
        "pop rbp",
        "pop rbx",
        "ret",
    );
}
