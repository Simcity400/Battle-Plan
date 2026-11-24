use super::{
    ACTIVE, BASE, CREDIT, DELIVERY, DELIVERY_CALLERS, ENABLED, Entry, NOTHING_PENDING, ORIGINAL,
    OutputKind, PENDING, Pending, ROCKET_DELIVERY, append, credit_entry, invalidate,
    observe_credit, observe_delivery, refuse_delivery, refuse_spending,
};
use engine_production::{line_output_sites::ROCKET_ADVANCE, line_output_sites::RocketTick};
use std::{
    cell::Cell,
    ops::Range,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

static NATIVE_FLIGHTS: AtomicUsize = AtomicUsize::new(0);
static WORLD_REFUSALS: AtomicUsize = AtomicUsize::new(0);
static ACQUIRED_EVENTS: AtomicUsize = AtomicUsize::new(0);
static RETAINED: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, Default)]
struct Scope {
    present: bool,
    tick: Option<RocketTick>,
    generation: Option<u64>,
}
thread_local! {
    static SCOPE: Cell<Scope> = const { Cell::new(Scope { present: false, tick: None, generation: None }) };
}

#[derive(Clone, Copy, Default)]
struct Delivery {
    country: u32,
    variant: u64,
    units: Option<i32>,
    coherent: bool,
}

#[derive(Clone, Copy)]
#[repr(C)]
struct Context {
    generation: Option<u64>,
    receiver: usize,
    second: usize,
    third: usize,
    caller: usize,
    kind: u32,
    active: bool,
    observing: bool,
    failed: bool,
    acquired: bool,
    scope_owned: bool,
    unknown_delivery: bool,
    refused: bool,
    tick: Option<RocketTick>,
    delivery: Option<Delivery>,
    pending: Pending,
    credit: Option<Entry>,
}
impl Default for Context {
    fn default() -> Self {
        Self {
            generation: None,
            receiver: 0,
            second: 0,
            third: 0,
            caller: 0,
            kind: 0,
            active: false,
            observing: false,
            failed: false,
            acquired: false,
            scope_owned: false,
            unknown_delivery: false,
            refused: false,
            tick: None,
            delivery: None,
            pending: NOTHING_PENDING,
            credit: None,
        }
    }
}

fn retain() {
    RETAINED.store(true, Ordering::SeqCst);
    ENABLED.store(false, Ordering::SeqCst);
    // SAFETY: Rocket observer hook failure invalidates the active session's line-output observation.
    unsafe { invalidate() };
}
pub(super) fn retention_required() -> bool {
    RETAINED.load(Ordering::SeqCst)
        || NATIVE_FLIGHTS.load(Ordering::SeqCst) != 0
        || ACQUIRED_EVENTS.load(Ordering::SeqCst) != 0
}
fn acquire(count: &AtomicUsize) -> bool {
    count
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |value| {
            value.checked_add(1)
        })
        .is_ok()
}

pub(super) fn unavailable_reason() -> Option<&'static str> {
    crate::world_lifetime::unavailable_reason()
}

pub(super) fn refusals() -> usize {
    WORLD_REFUSALS.load(Ordering::Relaxed)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DeliverySource {
    Equipment,
    Rocket,
    Unsupported,
}
fn delivery_source(caller: usize) -> DeliverySource {
    if caller == ROCKET_DELIVERY.load(Ordering::Relaxed) {
        DeliverySource::Rocket
    } else if DELIVERY_CALLERS
        .iter()
        .any(|address| address.load(Ordering::Relaxed) == caller)
    {
        DeliverySource::Equipment
    } else {
        DeliverySource::Unsupported
    }
}
fn rocket_source(kind: u32, caller: usize) -> bool {
    kind == 2
        || kind == 1
            && delivery_source(caller) == DeliverySource::Rocket
}
unsafe extern "system-unwind" fn before(
    pointer: *mut Context,
    receiver: usize,
    second: usize,
    third: usize,
    caller: usize,
    kind: u32,
) {
    let _timing = crate::hook_timing::enter(crate::hook_timing::Family::LineOutput);
    unsafe {
        pointer.write(Context {
            receiver,
            second,
            third,
            caller,
            kind,
            ..Default::default()
        })
    };
    let result = catch_unwind(AssertUnwindSafe(|| {
        let context = unsafe { &mut *pointer };
        if RETAINED.load(Ordering::SeqCst) || !acquire(&ACTIVE.0) {
            context.failed = true;
            retain();
            return;
        }
        context.active = true;
        context.observing = ENABLED.load(Ordering::SeqCst);
        context.generation = crate::world_lifetime::token();
        if kind == 0 {
            context.pending = PENDING.replace(NOTHING_PENDING);
            context.refused = !context.observing || context.generation.is_none();
            if !context.refused {
                context.credit = credit_entry(
                    receiver,
                    second as i64,
                    third as i32,
                    caller,
                    context.pending,
                );
            }
            return;
        }
        if !context.observing {
            context.refused = true;
            return;
        }
        if context.generation.is_none() {
            if rocket_source(context.kind, context.caller) {
                WORLD_REFUSALS.fetch_add(1, Ordering::Relaxed);
            }
            context.refused = true;
            return;
        }
        let base = BASE.load(Ordering::Relaxed);
        if kind == 1 {
            match delivery_source(caller) {
                DeliverySource::Equipment => return,
                DeliverySource::Rocket => {}
                DeliverySource::Unsupported => {
                    if !acquire(&ACQUIRED_EVENTS) {
                        context.failed = true;
                        retain();
                        return;
                    }
                    context.acquired = true;
                    context.unknown_delivery = true;
                    return;
                }
            }
        }
        if !acquire(&ACQUIRED_EVENTS) {
            context.failed = true;
            retain();
            return;
        }
        context.acquired = true;
        let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
        if kind == 2 {
            let occupied = SCOPE.try_with(|scope| scope.get().present);
            if !matches!(occupied, Ok(false)) {
                context.failed = true;
                retain();
                return;
            }
            context.tick = engine_production::line_output_sites::read_rocket_spending(
                &memory,
                base,
                receiver,
                &collector_observers::variant_stat_witness::ObserverStatWitness,
            );
            if SCOPE
                .try_with(|scope| {
                    scope.set(Scope {
                        present: true,
                        tick: context.tick,
                        generation: context.generation,
                    })
                })
                .is_err()
            {
                context.failed = true;
                retain();
                return;
            }
            context.scope_owned = true;
        } else {
            let scope = SCOPE.try_with(|scope| scope.get()).unwrap_or_default();
            let variant =
                engine_native_parts::equipment_variant_identity::identity(&memory, base, second).unwrap_or(0);
            if let crate::rocket_output::Entry::Delivery {
                country,
                variant,
                units,
                coherent,
            } = crate::rocket_output::Entry::delivery(scope.tick, variant, third as i64)
            {
                context.delivery = Some(Delivery {
                    country,
                    variant,
                    units,
                    coherent: scope.present && scope.generation == context.generation && coherent,
                });
            }
        }
    }));
    if let Err(payload) = result {
        std::mem::forget(payload);
        unsafe { (*pointer).failed = true };
        retain();
    }
}

unsafe extern "system-unwind" fn returned(pointer: *mut Context) -> u8 {
    let _timing = crate::hook_timing::enter_return(crate::hook_timing::Family::LineOutput);
    let context = unsafe { pointer.read() };
    if context.failed || !context.active {
        return 0;
    }
    let result = catch_unwind(AssertUnwindSafe(|| {
        if context.refused
            || !crate::world_lifetime::current(context.generation)
            || context.kind == 0
                && context.pending.producer.is_some()
                && context.pending.generation != context.generation
        {
            if !context.refused {
                if rocket_source(context.kind, context.caller) {
                    WORLD_REFUSALS.fetch_add(1, Ordering::Relaxed);
                }
                // SAFETY: Rocket observer hook failure invalidates the active session's line-output observation.
    unsafe { invalidate() };
            }
            PENDING.set(NOTHING_PENDING);
            if context.scope_owned && SCOPE.try_with(|scope| scope.set(Scope::default())).is_err() {
                retain();
                return false;
            }
            if context.acquired {
                ACQUIRED_EVENTS.fetch_sub(1, Ordering::SeqCst);
            }
            ACTIVE.0.fetch_sub(1, Ordering::SeqCst);
            return true;
        }
        match context.kind {
            0 if context.observing && ENABLED.load(Ordering::SeqCst) => {
                observe_credit(context.credit)
            }
            1 if context.observing && ENABLED.load(Ordering::SeqCst) => {
                if context.unknown_delivery {
                    refuse_delivery("production delivery caller unsupported");
                    append(Entry {
                        kind: OutputKind::UnknownDelivery,
                        ..Default::default()
                    });
                } else if let Some(delivery) = context.delivery {
                    if !delivery.coherent {
                        refuse_delivery(
                            "rocket delivery producer or design unreadable or mismatched",
                        );
                    } else if delivery.units.is_none() {
                        refuse_delivery("rocket delivery amount is not signed whole i32 units");
                    }
                    append(Entry {
                        kind: OutputKind::Delivery {
                            units_known: delivery.units.is_some(),
                        },
                        country: delivery.country,
                        variant: delivery.variant,
                        units: delivery.units.unwrap_or(0),
                        exact: delivery.coherent && delivery.units.is_some(),
                        ..Default::default()
                    });
                } else {
                    observe_delivery(context.caller, context.second, context.third as i64);
                }
            }
            2 => {
                if let Some(tick) = context.tick {
                    if let Some(reason) = tick.failure.reason() {
                        refuse_spending(reason);
                    }
                    append(Entry {
                        kind: OutputKind::Spending(tick.failure),
                        country: tick.country,
                        variant: tick.variant,
                        spending: tick
                            .failure
                            .reason()
                            .is_none()
                            .then_some((tick.spent_raw, tick.instant_raw)),
                        exact: tick.failure.reason().is_none(),
                        ..Default::default()
                    });
                }
                if context.scope_owned
                    && SCOPE.try_with(|scope| scope.set(Scope::default())).is_err()
                {
                    retain();
                    return false;
                }
            }
            0 | 1 => {}
            _ => {
                retain();
                return false;
            }
        }
        if context.acquired {
            ACQUIRED_EVENTS.fetch_sub(1, Ordering::SeqCst);
        }
        ACTIVE.0.fetch_sub(1, Ordering::SeqCst);
        true
    }));
    match result {
        Ok(true) => 1,
        Ok(false) => 0,
        Err(payload) => {
            std::mem::forget(payload);
            retain();
            0
        }
    }
}

const CONTEXT: usize = 0xa0;
const FRAME: usize = CONTEXT + std::mem::size_of::<Context>().div_ceil(16) * 16 + 8;
macro_rules! owner_entry {
    ($name:literal, $end:literal, $kind:expr, $original:ident, $offset:expr) => {
        core::arch::global_asm!(
            ".text", concat!(".globl ", $name), concat!(".seh_proc ", $name), concat!($name, ":"),
            "sub rsp, {frame}", ".seh_stackalloc {frame}", ".seh_endprologue",
            "mov [rsp + 0x30], rcx", "mov [rsp + 0x38], rdx",
            "mov [rsp + 0x40], r8", "mov [rsp + 0x48], r9",
            "mov qword ptr [rsp + 0x80], 0",
            "2:", "mov rax, [rip + {flight}]", "cmp rax, -1", "je 3f",
            "lea r10, [rax + 1]", "lock cmpxchg [rip + {flight}], r10", "jne 2b",
            "mov qword ptr [rsp + 0x80], 1", "jmp 4f",
            "3:", "mov al, 1", "xchg byte ptr [rip + {retained}], al",
            "4:", "mov rax, [rsp + {frame}]", "mov [rsp + 0x20], rax",
            "mov qword ptr [rsp + 0x28], {kind}",
            "mov r9, [rsp + 0x40]", "mov r8, [rsp + 0x38]", "mov rdx, [rsp + 0x30]",
            "lea rcx, [rsp + {context}]", "call {pre}",
            "mov rcx, [rsp + 0x30]", "mov rdx, [rsp + 0x38]",
            "mov r8, [rsp + 0x40]", "mov r9, [rsp + 0x48]",
            "call qword ptr [rip + {original} + {offset}]",
            "mov [rsp + 0x60], rax", "movdqu [rsp + 0x68], xmm0",
            "lea rcx, [rsp + {context}]", "call {post}",
            "test al, al", "jz 5f", "cmp qword ptr [rsp + 0x80], 0", "je 5f",
            "lock dec qword ptr [rip + {flight}]",
            "5:", "mov rax, [rsp + 0x60]", "movdqu xmm0, [rsp + 0x68]",
            "add rsp, {frame}", "ret", ".seh_endproc", concat!(".globl ", $end), concat!($end, ":"),
            frame = const FRAME, context = const CONTEXT, kind = const $kind,
            offset = const $offset, original = sym $original,
            pre = sym before, post = sym returned, flight = sym NATIVE_FLIGHTS, retained = sym RETAINED,
        );
    };
}
owner_entry!(
    "production_owned_credit",
    "production_owned_credit_end",
    0,
    ORIGINAL,
    0
);
owner_entry!(
    "production_owned_delivery",
    "production_owned_delivery_end",
    1,
    ORIGINAL,
    8
);
owner_entry!(
    "production_owned_rocket",
    "production_owned_rocket_end",
    2,
    ORIGINAL,
    16
);
unsafe extern "system-unwind" {
    fn production_owned_credit();
    fn production_owned_credit_end();
    fn production_owned_delivery();
    fn production_owned_delivery_end();
    fn production_owned_rocket();
    fn production_owned_rocket_end();
}

pub(super) fn descriptors() -> [(crate::output_hooks::Site, usize, crate::hook::TapSafety); 3] {
    let entry = |site, begin, end| {
        (
            site,
            begin,
            crate::hook::TapSafety {
                in_flight: &NATIVE_FLIGHTS,
                stub: begin..end,
            },
        )
    };
    [
        entry(
            (&CREDIT).into(),
            production_owned_credit as *const () as usize,
            production_owned_credit_end as *const () as usize,
        ),
        entry(
            (&DELIVERY).into(),
            production_owned_delivery as *const () as usize,
            production_owned_delivery_end as *const () as usize,
        ),
        entry(
            crate::output_hooks::Site {
                rva: ROCKET_ADVANCE.rva,
            },
            production_owned_rocket as *const () as usize,
            production_owned_rocket_end as *const () as usize,
        ),
    ]
}
pub(super) fn stub_ranges() -> [Range<usize>; 3] {
    descriptors().map(|(_, _, safety)| safety.stub)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_production::line_output_sites::ROCKET_DELIVERY_RETURN;

    #[test]
    fn changed_world_normal_return_discards_owned_payload_and_settles_its_flight() {
        let _lock = super::super::test_lock();
        super::super::observe_for_test(true);
        for mut context in [
            Context {
                generation: Some(2),
                active: true,
                acquired: true,
                kind: 2,
                tick: Some(RocketTick {
                    country: 7,
                    variant: 88,
                    spent_raw: 99,
                    ..Default::default()
                }),
                ..Default::default()
            },
            Context {
                generation: Some(1),
                active: true,
                acquired: true,
                kind: 0,
                pending: Pending {
                    generation: Some(2),
                    producer: Some(0),
                    ..NOTHING_PENDING
                },
                ..Default::default()
            },
        ] {
            ACTIVE.0.fetch_add(1, Ordering::SeqCst);
            ACQUIRED_EVENTS.fetch_add(1, Ordering::SeqCst);
            let refusals = refusals();
            assert_eq!(unsafe { returned(&raw mut context) }, 1);
            assert_eq!(ACTIVE.load(Ordering::SeqCst), 0);
            assert_eq!(ACQUIRED_EVENTS.load(Ordering::SeqCst), 0);
            assert_eq!(self::refusals(), refusals + usize::from(context.kind == 2));
            let mut entries = vec![Entry::default(); 1];
            assert_eq!(super::super::drain(&mut entries).unwrap().copied, 0);
        }
        super::super::observe_for_test(false);
    }

    #[test]
    fn credit_publishes_what_its_entry_read_after_a_nested_refresh() {
        use super::super::tests::{Line, MILITARY};
        use replay_data::rows::OutputPurpose;
        let _lock = super::super::test_lock();
        super::super::observe_for_test(true);
        let mut line = Line::new(0x11_0000_0021, 0x5_0000_0046, 0x9_0000_004f);
        line.set_converting(true);
        let caller = BASE.load(Ordering::Relaxed) + CREDIT.callers[MILITARY].0;
        let mut context = Context::default();
        unsafe { before(&raw mut context, line.address(), 7, 0, caller, 0) };
        line.set_converting(false);
        line.replace(0x6_0000_0046, 0x8_0000_004f);
        assert_eq!(unsafe { returned(&raw mut context) }, 1);
        let mut entries = vec![Entry::default(); 2];
        assert_eq!(super::super::drain(&mut entries).unwrap().copied, 1);
        assert_eq!(
            (entries[0].purpose, entries[0].variant, entries[0].mio),
            (OutputPurpose::Converted, 0x5_0000_0046, 0x9_0000_004f)
        );
        assert_eq!(ACTIVE.load(Ordering::SeqCst), 0);
        super::super::observe_for_test(false);
    }

    #[test]
    fn sole_delivery_owner_distinguishes_both_native_families_and_unsupported() {
        let _lock = super::super::test_lock();
        let base = 0x140000000;
        super::super::bind_callers(base).unwrap();
        for caller in DELIVERY.callers {
            assert_eq!(delivery_source(base + caller.0), DeliverySource::Equipment);
        }
        assert_eq!(
            delivery_source(base + ROCKET_DELIVERY_RETURN.0),
            DeliverySource::Rocket
        );
        for caller in [base - 1, base, base + ROCKET_DELIVERY_RETURN.0 + 1] {
            assert_eq!(delivery_source(caller), DeliverySource::Unsupported);
        }
        super::super::bind_callers(0).unwrap();
    }

    #[test]
    fn preboundary_equipment_calls_keep_the_guard_without_counting_as_rocket_refusals() {
        let _lock = super::super::test_lock();
        super::super::observe_for_test(true);
        crate::world_lifetime::observe_for_test(false);
        let base = BASE.load(Ordering::Relaxed);
        for (kind, caller, rocket) in [
            (1, base + DELIVERY.callers[0].0, false),
            (0, base + CREDIT.callers[0].0, false),
            (1, base + ROCKET_DELIVERY_RETURN.0, true),
            (2, 0, true),
        ] {
            let prior = refusals();
            let mut context = Context::default();
            unsafe { before(&raw mut context, 0, 0, 0, caller, kind) };
            assert!(context.refused && !context.acquired);
            assert_eq!(unsafe { returned(&raw mut context) }, 1);
            assert_eq!(refusals(), prior + usize::from(rocket));
            assert_eq!(ACTIVE.load(Ordering::SeqCst), 0);
            assert_eq!(ACQUIRED_EVENTS.load(Ordering::SeqCst), 0);
        }
        let mut entries = vec![Entry::default(); 1];
        assert_eq!(
            super::super::drain(&mut entries).err(),
            Some("native world observation is disabled")
        );
        crate::world_lifetime::observe_for_test(true);
        assert_eq!(super::super::drain(&mut entries).unwrap().copied, 0);
        super::super::observe_for_test(false);
    }

    #[test]
    fn owner_stubs_forward_arguments_and_integer_or_float_results_once() {
        static CALLS: AtomicUsize = AtomicUsize::new(0);
        extern "system" fn integer(first: usize, second: usize, third: usize) -> usize {
            CALLS.fetch_add(1, Ordering::Relaxed);
            first ^ second.rotate_left(7) ^ third.rotate_left(13)
        }
        extern "system" fn floating(first: usize, second: usize, third: usize) -> f64 {
            CALLS.fetch_add(1, Ordering::Relaxed);
            first as f64 + 2.0 * second as f64 + 4.0 * third as f64
        }
        let _lock = super::super::test_lock();
        super::super::observe_for_test(false);
        let counters = super::super::COUNTERS
            .each_ref()
            .map(|count| count.load(Ordering::Relaxed));
        CALLS.store(0, Ordering::Relaxed);
        for (index, (_, stub, _)) in descriptors().into_iter().enumerate() {
            let previous = ORIGINAL[index].swap(integer as *const () as usize, Ordering::SeqCst);
            let call: unsafe extern "system" fn(usize, usize, usize) -> usize =
                unsafe { std::mem::transmute(stub) };
            assert_eq!(
                unsafe { call(41, 17, 9) },
                41 ^ 17usize.rotate_left(7) ^ 9usize.rotate_left(13)
            );
            ORIGINAL[index].store(floating as *const () as usize, Ordering::SeqCst);
            let call: unsafe extern "system" fn(usize, usize, usize) -> f64 =
                unsafe { std::mem::transmute(stub) };
            assert_eq!(unsafe { call(41, 17, 9) }, 111.0);
            ORIGINAL[index].store(previous, Ordering::SeqCst);
        }
        let line = super::super::tests::Line::new(0x11_0000_0021, 0x5_0000_0046, 0x9_0000_004f);
        for enabled_at_entry in [false, true] {
            super::super::observe_for_test(enabled_at_entry);
            let caller = BASE.load(Ordering::Relaxed) + CREDIT.callers[super::super::tests::MILITARY].0;
            let mut context = Context::default();
            unsafe { before(&raw mut context, line.address(), 17, 9, caller, 0) };
            super::super::observe_for_test(!enabled_at_entry);
            assert_eq!(unsafe { returned(&raw mut context) }, 1);
        }
        super::super::observe_for_test(false);
        assert_eq!(CALLS.load(Ordering::Relaxed), 6);
        assert_eq!(NATIVE_FLIGHTS.load(Ordering::SeqCst), 0);
        assert_eq!(ACTIVE.load(Ordering::SeqCst), 0);
        assert_eq!(ACQUIRED_EVENTS.load(Ordering::SeqCst), 0);
        assert_eq!(
            super::super::COUNTERS
                .each_ref()
                .map(|count| count.load(Ordering::Relaxed)),
            counters
        );
    }

    #[test]
    fn unsupported_normal_delivery_is_an_owned_coverage_marker_not_just_a_metric() {
        let _lock = super::super::test_lock();
        super::super::observe_for_test(true);
        let before_refusals = super::super::DELIVERY_REFUSALS.load(Ordering::Relaxed);
        let mut context = Context::default();
        unsafe { before(&raw mut context, 0, 0, 123, 0xfeed, 1) };
        assert!(context.acquired && context.unknown_delivery && context.active);
        assert_eq!(unsafe { returned(&raw mut context) }, 1);
        assert_eq!(ACQUIRED_EVENTS.load(Ordering::SeqCst), 0);
        assert_eq!(ACTIVE.load(Ordering::SeqCst), 0);
        let mut entries = vec![Entry::default(); 1];
        let drained = super::super::drain(&mut entries).expect("settled marker drains");
        assert_eq!(drained.copied, 1);
        assert_eq!(entries[0].kind, OutputKind::UnknownDelivery);
        assert_eq!(
            (entries[0].line, entries[0].country, entries[0].variant),
            (0, 0, 0)
        );
        assert_eq!(
            super::super::DELIVERY_REFUSALS.load(Ordering::Relaxed),
            before_refusals + 1
        );
        super::super::observe_for_test(false);
    }
}
