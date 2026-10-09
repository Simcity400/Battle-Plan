//! Native invalidation, not a load-success detector. A new occurrence starts
//! at the next existing hourly boundary; its first observer interval is partial.
use crate::hook::{Hook, Registers, Tap, TapSafety};
use engine::{Memory, memory::read, world_lifetime_sites as native};
use std::{
    ops::Range,
    sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};

struct Signal {
    generation: AtomicU64,
    admitted: AtomicU64,
}
impl Signal {
    const fn new() -> Self {
        Self {
            generation: AtomicU64::new(1),
            admitted: AtomicU64::new(0),
        }
    }
    fn invalidate(&self) {
        self.admitted.store(0, Ordering::SeqCst);
        // Exhaustion stays closed rather than wrapping into an old occurrence.
        let generation = self.generation.load(Ordering::SeqCst);
        if self
            .generation
            .compare_exchange(
                generation,
                generation.saturating_add(1),
                Ordering::SeqCst,
                Ordering::SeqCst,
            )
            .is_err()
        {
            // Never retry on a native writer. A racing invalidation closes the
            // attachment rather than waiting to order competing world changes.
            self.generation.store(u64::MAX, Ordering::SeqCst);
        }
    }
    // HourlyUpdate@1401DCEF0 entry admits the occurrence read with its clock.
    fn boundary(&self, generation: u64, valid_clock: bool) {
        if !valid_clock {
            self.admitted.store(0, Ordering::SeqCst);
        } else if generation != u64::MAX && generation == self.generation.load(Ordering::SeqCst) {
            self.admitted.store(generation, Ordering::SeqCst);
        }
    }
    fn token(&self) -> Option<u64> {
        let admitted = self.admitted.load(Ordering::SeqCst);
        (admitted != 0 && admitted == self.generation.load(Ordering::SeqCst)).then_some(admitted)
    }
}
static SIGNAL: Signal = Signal::new();
static ENABLED: AtomicBool = AtomicBool::new(false);
pub(crate) static ACTIVE: AtomicUsize = AtomicUsize::new(0);
static DELETE_ORIGINAL: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn generation() -> u64 {
    SIGNAL.generation.load(Ordering::SeqCst)
}
pub(crate) fn token() -> Option<u64> {
    #[cfg(test)]
    if let Some(token) = TEST_TOKEN.with(|token| token.get()) {
        return token;
    }
    ENABLED
        .load(Ordering::SeqCst)
        .then(|| SIGNAL.token())
        .flatten()
}
#[cfg(test)]
thread_local! { static TEST_TOKEN: std::cell::Cell<Option<Option<u64>>> = const { std::cell::Cell::new(None) }; }
#[cfg(test)]
pub(crate) fn observe_for_test(enabled: bool) {
    TEST_TOKEN.with(|token| token.set(Some(enabled.then_some(1))));
}
pub(crate) fn current(token: Option<u64>) -> bool {
    token.is_some() && token == self::token()
}
pub(crate) fn boundary(generation: u64, valid_clock: bool) {
    SIGNAL.boundary(generation, valid_clock && ENABLED.load(Ordering::SeqCst));
}
pub(crate) fn invalidating() -> bool {
    ENABLED.load(Ordering::SeqCst) && SIGNAL.token().is_none()
}
pub(crate) fn unavailable_reason() -> Option<&'static str> {
    observation_unavailable(ENABLED.load(Ordering::SeqCst), generation(), token())
}
fn observation_unavailable(
    enabled: bool,
    generation: u64,
    token: Option<u64>,
) -> Option<&'static str> {
    if generation == u64::MAX {
        return Some("native world occurrence exhausted or concurrent invalidation");
    }
    token.is_none().then_some(if enabled {
        "native world observation has not reached an hourly boundary"
    } else {
        "native world observation is disabled"
    })
}
pub(crate) fn status() -> impl serde::Serialize {
    crate::status::status!({
        "generation": generation(), "enabled": ENABLED.load(Ordering::SeqCst),
        "admitted": token(), "unavailable": unavailable_reason(),
    })
}
unsafe extern "system-unwind" fn observe(_: &Registers, _: usize) {
    SIGNAL.invalidate();
}
extern "system" fn deleting() {
    SIGNAL.invalidate();
}

// Tail transfer leaves no DLL return PC in the native destructor. Only the
// short pre-observer and its entry/exit need a flight; native exceptions then
// unwind the authenticated original prologue with the original caller stack.
core::arch::global_asm!(
    ".text", ".globl world_delete_entry", ".seh_proc world_delete_entry", "world_delete_entry:",
    "sub rsp, 0xc8", ".seh_stackalloc 0xc8", ".seh_endprologue",
    "mov [rsp+0x20], rax", "mov [rsp+0x28], rcx", "mov [rsp+0x30], rdx",
    "mov [rsp+0x38], r8", "mov [rsp+0x40], r9", "mov [rsp+0x48], r10", "mov [rsp+0x50], r11",
    "movdqu [rsp+0x60], xmm0", "movdqu [rsp+0x70], xmm1", "movdqu [rsp+0x80], xmm2",
    "movdqu [rsp+0x90], xmm3", "movdqu [rsp+0xa0], xmm4", "movdqu [rsp+0xb0], xmm5",
    "lock inc qword ptr [rip+{active}]", "call {observer}",
    "lock dec qword ptr [rip+{active}]",
    "movdqu xmm0, [rsp+0x60]", "movdqu xmm1, [rsp+0x70]", "movdqu xmm2, [rsp+0x80]",
    "movdqu xmm3, [rsp+0x90]", "movdqu xmm4, [rsp+0xa0]", "movdqu xmm5, [rsp+0xb0]",
    "mov rax, [rsp+0x20]", "mov rcx, [rsp+0x28]", "mov rdx, [rsp+0x30]",
    "mov r8, [rsp+0x38]", "mov r9, [rsp+0x40]", "mov r10, [rsp+0x48]", "mov r11, [rsp+0x50]",
    ".globl world_delete_epilogue", "world_delete_epilogue:",
    "add rsp, 0xc8", "jmp qword ptr [rip+{original}]", ".seh_endproc",
    ".globl world_delete_end", "world_delete_end:",
    active = sym ACTIVE, observer = sym deleting, original = sym DELETE_ORIGINAL,
);
unsafe extern "system" {
    fn world_delete_entry();
    fn world_delete_end();
}
#[cfg(test)]
unsafe extern "system" {
    fn world_delete_epilogue();
}

pub(crate) struct Hooks {
    taps: [Tap; 2],
    delete: Hook,
}
impl Hooks {
    pub(crate) unsafe fn new(
        memory: &impl Memory,
        base: usize,
        generation: u64,
    ) -> Result<Self, &'static str> {
        if generation == u64::MAX {
            return Err("native world occurrence exhausted or concurrent invalidation");
        }
        SIGNAL.generation.store(generation.max(1), Ordering::SeqCst);
        for site in [&native::RESET, &native::CONSTRUCT, &native::DELETE] {
            if read::<[u8; 16]>(memory, base + site.rva) != Some(site.sentinel) {
                return Err("native world lifetime sentinel differs");
            }
        }
        let tap = |site: &native::Site| unsafe {
            Tap::new(base + site.rva, observe, 0, site.sentinel)
                .map_err(|_| "native world lifetime tap construction failed")
        };
        let taps = [tap(&native::RESET)?, tap(&native::CONSTRUCT)?];
        let delete = unsafe {
            Hook::new_with_safety(
                base + native::DELETE.rva,
                world_delete_entry as *const () as usize,
                native::DELETE.sentinel,
                TapSafety {
                    in_flight: &ACTIVE,
                    stub: delete_range(),
                },
            )
        }
        .map_err(|_| "native world deletion hook construction failed")?;
        DELETE_ORIGINAL.store(delete.trampoline(), Ordering::SeqCst);
        Ok(Self { taps, delete })
    }
    pub(crate) fn set_enabled(&mut self, enabled: bool) -> Result<bool, &'static str> {
        if !enabled {
            ENABLED.store(false, Ordering::SeqCst);
            SIGNAL.admitted.store(0, Ordering::SeqCst);
        }
        let [reset, construct] = &mut self.taps;
        let ready = Hook::set_enabled_batch(
            &mut [
                reset.publication_hook(),
                construct.publication_hook(),
                &mut self.delete,
            ],
            enabled,
        )
        .map_err(|_| "native world lifetime hook state change failed")?;
        if ready {
            ENABLED.store(enabled, Ordering::SeqCst);
        }
        Ok(ready)
    }
    pub(crate) fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.taps
            .iter()
            .flat_map(Tap::executable_ranges)
            .chain(self.delete.executable_ranges())
            .chain(std::iter::once(delete_range()))
    }
}
fn delete_range() -> Range<usize> {
    world_delete_entry as *const () as usize..world_delete_end as *const () as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::System::Diagnostics::Debug::{
        CONTEXT, RtlLookupFunctionEntry, RtlVirtualUnwind,
    };

    #[test]
    fn deleting_stub_unwinds_to_the_original_caller_stack() {
        let entry = world_delete_entry as *const () as usize;
        assert_eq!(
            unsafe { std::slice::from_raw_parts(entry as *const u8, 7) },
            &[0x48, 0x81, 0xec, 0xc8, 0, 0, 0]
        );
        #[repr(C, align(16))]
        struct AlignedContext(CONTEXT);
        #[repr(align(16))]
        struct Stack([u64; 32]);
        let mut stack = Stack([0; 32]);
        stack.0[25] = 0x1234_5678;
        let epilogue = world_delete_epilogue as *const () as usize;
        for (pc, rsp) in [
            (entry, stack.0.as_ptr() as u64 + 200),
            (entry + 7, stack.0.as_ptr() as u64),
            (epilogue, stack.0.as_ptr() as u64),
            (epilogue + 7, stack.0.as_ptr() as u64 + 200),
        ] {
            let mut image_base = 0;
            let function =
                unsafe { RtlLookupFunctionEntry(pc as u64, &mut image_base, std::ptr::null_mut()) };
            assert!(!function.is_null());
            let mut context = AlignedContext(unsafe { std::mem::zeroed() });
            context.0.Rip = pc as u64;
            context.0.Rsp = rsp;
            context.0.Rbx = 7;
            context.0.Rdi = 8;
            let mut handler_data = std::ptr::null_mut();
            let mut establisher = 0;
            unsafe {
                RtlVirtualUnwind(
                    0,
                    image_base,
                    pc as u64,
                    function,
                    &mut context.0,
                    &mut handler_data,
                    &mut establisher,
                    std::ptr::null_mut(),
                );
            }
            assert_eq!(context.0.Rip, stack.0[25], "pc {pc:x}");
            assert_eq!(context.0.Rsp, stack.0.as_ptr() as u64 + 208, "pc {pc:x}");
            assert_eq!((context.0.Rbx, context.0.Rdi), (7, 8));
        }
    }
    #[test]
    fn reset_and_address_reuse_revoke_an_occurrence_before_the_next_boundary() {
        let signal = Signal::new();
        assert_eq!(signal.token(), None);
        signal.boundary(1, true);
        let before = signal.token().unwrap();
        signal.invalidate();
        assert_eq!(signal.token(), None);
        signal.boundary(2, true);
        assert_ne!(signal.token(), Some(before));
        signal.invalidate(); // deletion followed by construction can invalidate twice
        signal.invalidate();
        assert_eq!(signal.token(), None);
        signal.boundary(4, true);
        assert_eq!(signal.token(), Some(before + 3));
    }
    #[test]
    fn exhaustion_cannot_reopen_or_reuse_an_old_occurrence() {
        let signal = Signal {
            generation: AtomicU64::new(u64::MAX - 1),
            admitted: AtomicU64::new(0),
        };
        signal.boundary(u64::MAX - 1, true);
        signal.invalidate();
        signal.invalidate();
        signal.boundary(u64::MAX, true);
        assert_eq!(signal.token(), None);
    }
    #[test]
    fn invalidation_during_clock_read_cannot_admit_the_replacement() {
        let signal = Signal::new();
        signal.invalidate();
        signal.boundary(1, true);
        assert_eq!(signal.token(), None);
        signal.boundary(2, true);
        assert_eq!(signal.token(), Some(2));
    }

    #[test]
    fn load_invalidations_admit_at_the_first_clean_boundary_and_detach_reports_disabled() {
        let signal = Signal::new();
        let generation = || signal.generation.load(Ordering::SeqCst);
        for _ in 0..4 {
            let reading = generation();
            signal.invalidate();
            signal.boundary(reading, true);
            signal.boundary(generation(), false);
            assert_eq!(signal.token(), None);
        }
        signal.boundary(generation(), true);
        assert_eq!(signal.token(), Some(generation()));
        assert_eq!(
            observation_unavailable(true, generation(), signal.token()),
            None
        );
        signal.boundary(generation(), false);
        assert_eq!(
            observation_unavailable(false, generation(), signal.token()),
            Some("native world observation is disabled")
        );
    }
}
