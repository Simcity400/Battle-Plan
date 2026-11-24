use crate::hook::{Hook, Registers, Tap, TapSafety};
use engine_mutations::world_lifetime_sites as native;
use engine_native_memory::Memory;
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
            self.generation.store(u64::MAX, Ordering::SeqCst);
        }
    }
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
    fn observed_generation(&self, enabled: bool) -> Option<u64> {
        let generation = self.generation.load(Ordering::SeqCst);
        (enabled && generation != u64::MAX).then_some(generation)
    }
}
static SIGNAL: Signal = Signal::new();
static ENABLED: AtomicBool = AtomicBool::new(false);
pub static ACTIVE: AtomicUsize = AtomicUsize::new(0);
static DELETE_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static STATE_RELOAD_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static STRAITS_RELOAD_ORIGINAL: AtomicUsize = AtomicUsize::new(0);

pub fn generation() -> u64 {
    SIGNAL.generation.load(Ordering::SeqCst)
}
pub fn observed_generation() -> Option<u64> {
    SIGNAL.observed_generation(ENABLED.load(Ordering::SeqCst))
}
pub fn token() -> Option<u64> {
    #[cfg(any(test, feature = "test-support"))]
    if let Some(token) = TEST_TOKEN.with(|token| token.get()) {
        return token;
    }
    ENABLED
        .load(Ordering::SeqCst)
        .then(|| SIGNAL.token())
        .flatten()
}
#[cfg(any(test, feature = "test-support"))]
thread_local! { static TEST_TOKEN: std::cell::Cell<Option<Option<u64>>> = const { std::cell::Cell::new(None) }; }
#[cfg(any(test, feature = "test-support"))]
pub fn invalidate_for_test() {
    SIGNAL.invalidate();
}
#[cfg(any(test, feature = "test-support"))]
pub fn observe_for_test(enabled: bool) {
    TEST_TOKEN.with(|token| token.set(Some(enabled.then_some(1))));
}
pub fn current(token: Option<u64>) -> bool {
    token.is_some() && token == self::token()
}
pub fn boundary(generation: u64, valid_clock: bool) {
    SIGNAL.boundary(generation, valid_clock && ENABLED.load(Ordering::SeqCst));
}
pub fn invalidating() -> bool {
    ENABLED.load(Ordering::SeqCst) && SIGNAL.token().is_none()
}
pub fn unavailable_reason() -> Option<&'static str> {
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
unsafe extern "system-unwind" fn observe(_: &Registers, _: usize) {
    let _timing = crate::hook_timing::enter(crate::hook_timing::Family::WorldLifetime);
    SIGNAL.invalidate();
}
extern "system" fn deleting() {
    let _timing = crate::hook_timing::enter(crate::hook_timing::Family::WorldLifetime);
    SIGNAL.invalidate();
}
unsafe extern "system" fn state_reload(editor: usize, b: usize, c: usize, d: usize) -> usize {
    unsafe { reload(&STATE_RELOAD_ORIGINAL, editor, b, c, d) }
}
unsafe extern "system" fn straits_reload(map: usize, b: usize, c: usize, d: usize) -> usize {
    unsafe { reload(&STRAITS_RELOAD_ORIGINAL, map, b, c, d) }
}
/// # Safety
/// The original passed here is a live function with the hooked function's ABI that accepts these
/// arguments, and every object they reach stays valid for the reads its site makes; one the
/// original destroys is an identity only once it returns.
unsafe fn reload(original: &AtomicUsize, a: usize, b: usize, c: usize, d: usize) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::WorldLifetime);
    ACTIVE.fetch_add(1, Ordering::SeqCst);
    SIGNAL.invalidate();
    let original: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
        unsafe { std::mem::transmute(original.load(Ordering::Acquire)) };
    let result = timing.original(|| unsafe { original(a, b, c, d) });
    SIGNAL.invalidate();
    ACTIVE.fetch_sub(1, Ordering::SeqCst);
    result
}

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

pub struct Hooks {
    taps: [Tap; 2],
    delete: Hook,
    state_reload: Hook,
    straits_reload: Hook,
}
impl Hooks {
    /// # Safety
    /// The memory source and base must identify the authenticated executable in
    /// this process. The caller owns this family exclusively and must restore
    /// its hooks and quiesce native calls before releasing code or unloading.
    pub unsafe fn new(
        _memory: &impl Memory,
        base: usize,
        generation: u64,
    ) -> Result<Self, &'static str> {
        if generation == u64::MAX {
            return Err("native world occurrence exhausted or concurrent invalidation");
        }
        SIGNAL.generation.store(generation.max(1), Ordering::SeqCst);
        let locate = |site: &native::Site| {
            engine_sites::hook(base, site.rva)
                .and_then(|code| Ok((code.address, code.sentinel::<16>()?)))
        };
        let located = (|| {
            Ok::<_, &'static str>([
                locate(&native::RESET)?,
                locate(&native::CONSTRUCT)?,
                locate(&native::STATE_RELOAD)?,
                locate(&native::STRAITS_RELOAD)?,
                locate(&native::DELETE)?,
            ])
        })();
        let [reset, construct, state_site, straits_site, delete_site] = match located {
            Ok(located) => located,
            Err(cause) => {
                crate::hook::installation::refused(
                    crate::hook::installation::Kind::Sentinel,
                    None,
                    cause,
                );
                return Err(cause);
            }
        };
        let tap = |(site, sentinel): (usize, [u8; 16])| unsafe {
            Tap::new(site, observe, 0, sentinel)
                .map_err(|_| "native world lifetime tap construction failed")
        };
        let taps = [tap(reset)?, tap(construct)?];
        let delete = unsafe {
            Hook::new_with_safety(
                delete_site.0,
                world_delete_entry as *const () as usize,
                delete_site.1,
                TapSafety {
                    in_flight: &ACTIVE,
                    stub: delete_range(),
                },
            )
        }
        .map_err(|_| "native world deletion hook construction failed")?;
        DELETE_ORIGINAL.store(delete.trampoline(), Ordering::SeqCst);
        let state_reload = unsafe {
            Hook::new(
                state_site.0,
                state_reload as *const () as usize,
                state_site.1,
            )
        }
        .map_err(|_| "native state reload hook construction failed")?;
        STATE_RELOAD_ORIGINAL.store(state_reload.trampoline(), Ordering::SeqCst);
        let straits_reload = unsafe {
            Hook::new(
                straits_site.0,
                straits_reload as *const () as usize,
                straits_site.1,
            )
        }
        .map_err(|_| "native straits reload hook construction failed")?;
        STRAITS_RELOAD_ORIGINAL.store(straits_reload.trampoline(), Ordering::SeqCst);
        Ok(Self {
            taps,
            delete,
            state_reload,
            straits_reload,
        })
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, &'static str> {
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
                &mut self.state_reload,
                &mut self.straits_reload,
            ],
            enabled,
        )
        .map_err(|_| "native world lifetime hook state change failed")?;
        if ready {
            ENABLED.store(enabled, Ordering::SeqCst);
        }
        Ok(ready)
    }
    pub fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.taps
            .iter()
            .flat_map(Tap::executable_ranges)
            .chain(self.delete.executable_ranges())
            .chain(self.state_reload.executable_ranges())
            .chain(self.straits_reload.executable_ranges())
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
    fn deletion_flights_balance_before_native_early_return_and_reentry() {
        struct Probe {
            native_entries: usize,
            native_flights: usize,
        }
        extern "system" fn native(probe: *mut Probe, reenter: usize) -> *mut Probe {
            let state = unsafe { &mut *probe };
            state.native_entries += 1;
            state.native_flights |= ACTIVE.load(Ordering::SeqCst);
            if reenter == 0 {
                return probe;
            }
            unsafe { invoke(probe, 0) }
        }
        unsafe fn invoke(probe: *mut Probe, reenter: usize) -> *mut Probe {
            let entry: unsafe extern "system" fn(*mut Probe, usize) -> *mut Probe =
                unsafe { std::mem::transmute(world_delete_entry as *const ()) };
            unsafe { entry(probe, reenter) }
        }
        struct Original(usize);
        impl Drop for Original {
            fn drop(&mut self) {
                DELETE_ORIGINAL.store(self.0, Ordering::SeqCst);
            }
        }
        let _guard = crate::hook::FAMILY_TEST
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _original =
            Original(DELETE_ORIGINAL.swap(native as *const () as usize, Ordering::SeqCst));
        assert_eq!(ACTIVE.load(Ordering::SeqCst), 0);
        for reenter in [0, 1] {
            let mut probe = Probe {
                native_entries: 0,
                native_flights: 0,
            };
            let result = unsafe { invoke(&raw mut probe, reenter) };
            assert_eq!(result, &raw mut probe);
            assert_eq!(probe.native_entries, reenter + 1);
            assert_eq!(probe.native_flights, 0);
            assert_eq!(ACTIVE.load(Ordering::SeqCst), 0);
        }
    }

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
                unsafe { RtlLookupFunctionEntry(pc as u64, &raw mut image_base, std::ptr::null_mut()) };
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
                    &raw mut context.0,
                    &raw mut handler_data,
                    &raw mut establisher,
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
        signal.invalidate();
        signal.invalidate();
        assert_eq!(signal.token(), None);
        signal.boundary(4, true);
        assert_eq!(signal.token(), Some(before + 3));
    }
    #[test]
    fn command_generation_precedes_hourly_admission_without_admitting_other_observers() {
        let signal = Signal::new();
        assert_eq!(signal.observed_generation(false), None);
        assert_eq!(signal.observed_generation(true), Some(1));
        assert_eq!(signal.token(), None);
        signal.invalidate();
        assert_eq!(signal.observed_generation(true), Some(2));
        assert_eq!(signal.token(), None);
        signal.boundary(2, true);
        assert_eq!(signal.token(), signal.observed_generation(true));
        signal.generation.store(u64::MAX, Ordering::SeqCst);
        assert_eq!(signal.observed_generation(true), None);
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
