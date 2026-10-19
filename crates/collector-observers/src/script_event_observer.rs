//! Script occurrences join the event feed at native fire/selection boundaries.
//! There is no queue census and no added hourly native acquisition.

use crate::{
    event_feed_history::ScriptFact,
    event_feed_observer as feed,
    hook::{Hook, Registers, Tap},
};
use engine_decisions::cevent;
use engine_events::script_event_sites as native;
use engine_native::{Memory, msvc_string::Descriptor};
use replay_data::event_feed_rows::{EventFeedScript, EventFeedSource};
use std::sync::atomic::{AtomicUsize, Ordering};
mod decisions;
mod delayed;
mod lifecycle;

static BASE: AtomicUsize = AtomicUsize::new(0);
static REGISTER_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static REGISTRY_ORIGINAL: [AtomicUsize; 3] = [const { AtomicUsize::new(0) }; 3];
static ACTIVE: AtomicUsize = AtomicUsize::new(0);
static DECISION_ORIGINAL: [AtomicUsize; 5] = [const { AtomicUsize::new(0) }; 5];
static ACTIVATION_ORIGINAL: AtomicUsize = AtomicUsize::new(0);

const RETAIN_NATIVE_FLIGHT: usize = 1 << 63;
pub(crate) struct Flight {
    normal: bool,
}
impl Flight {
    pub(crate) fn enter() -> Self {
        ACTIVE.fetch_add(1, Ordering::SeqCst);
        Self { normal: false }
    }
    pub(crate) fn returned(&mut self) {
        self.normal = true;
    }
}
impl Drop for Flight {
    fn drop(&mut self) {
        if self.normal {
            ACTIVE.fetch_sub(1, Ordering::SeqCst);
        } else {
            ACTIVE.fetch_or(RETAIN_NATIVE_FLIGHT, Ordering::SeqCst);
        }
    }
}
pub fn active() -> usize {
    ACTIVE.load(Ordering::SeqCst)
}

/// Record a reached execution or writer boundary. Effect invocation carries
/// no invented lifecycle/duration result; terminal operands come from stores.
pub fn decision(
    source: EventFeedSource,
    definition: usize,
    country: usize,
    selector: Option<usize>,
) {
    decision_result(source, definition, country, selector, None, None);
}
fn decision_result(
    source: EventFeedSource,
    definition: usize,
    country: usize,
    selector: Option<usize>,
    lifecycle: Option<i32>,
    days: Option<i32>,
) {
    feed::record_script(|world, memory, base| {
        if definition == 0
            || engine_native::read::<u64>(memory, definition)
                != base
                    .checked_add(engine_decisions::cdecision::VTABLE_RVA)
                    .map(|at| at as u64)
        {
            return Err("decision occurrence definition vtable unreadable or differs");
        }
        let actor = world.country_object(country)?;
        let token = Descriptor::read(
            memory,
            definition
                .checked_add(engine_decisions::cdecision::TOKEN)
                .ok_or("decision occurrence token address overflow")?,
        )?;
        let mut script = EventFeedScript {
            lifecycle,
            days,
            ..Default::default()
        };
        let target = if let Some(selector) = selector {
            let instance = selector
                .checked_sub(engine_decisions::ctargeted_decision::SELECTOR_VIEW)
                .ok_or("decision selector instance underflow")?;
            if engine_native::read::<u64>(memory, instance)
                != base
                    .checked_add(engine_decisions::ctargeted_decision::VTABLE_RVA)
                    .map(|at| at as u64)
                || field::<u64>(
                    memory,
                    instance,
                    engine_decisions::ctargeted_decision::DEFINITION,
                )? as usize
                    != definition
            {
                return Err("decision selector instance identity differs");
            }
            let country = field::<i32>(
                memory,
                selector,
                engine_decisions::ctargeted_decision::SELECTOR_COUNTRY,
            )?;
            script.country_target = Some(country as u32);
            if country > 0 {
                world.country(country as u32)?
            } else {
                script.numeric_target = Some(field(
                    memory,
                    selector,
                    engine_decisions::ctargeted_decision::SELECTOR_NUMERIC,
                )?);
                world.country(0)?
            }
        } else {
            world.country(0)?
        };
        Ok(Some(ScriptFact {
            source,
            actor,
            target,
            token,
            scope: None,
            dispatch_frame: 0,
            dispatch_definition: 0,
            cause: crate::war_provenance::inside(),
            script,
        }))
    });
}

macro_rules! decision_entry {
    ($name:ident, $index:literal, $source:expr, $targeted:literal) => {
        unsafe extern "system-unwind" fn $name(a: usize, b: usize, c: usize) -> usize {
            let mut flight = Flight::enter();
            let timing = crate::hook_timing::enter(crate::hook_timing::Family::EventFeedProvenance);
            decision($source, a, b, $targeted.then_some(c));
            type Original = unsafe extern "system-unwind" fn(usize, usize, usize) -> usize;
            let original: Original =
                unsafe { std::mem::transmute(DECISION_ORIGINAL[$index].load(Ordering::Acquire)) };
            let result = timing.original(|| unsafe { original(a, b, c) });
            flight.returned();
            result
        }
    };
}
decision_entry!(
    cancel,
    0,
    EventFeedSource::DecisionCancellationEffect,
    false
);
decision_entry!(remove, 1, EventFeedSource::DecisionRemovalEffect, false);
decision_entry!(
    targeted_cancel,
    2,
    EventFeedSource::DecisionCancellationEffect,
    true
);
decision_entry!(
    targeted_remove,
    3,
    EventFeedSource::DecisionRemovalEffect,
    true
);
unsafe extern "system-unwind" fn targeted_execute(
    definition: usize,
    country: usize,
    selector: usize,
) -> usize {
    let mut flight = Flight::enter();
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::EventFeedProvenance);
    let memory = unsafe { engine_native::memory::LiveMemory::new() };
    // Both native callers pass the instance's selector view. Selection still
    // has state zero; the timed updater stores its terminal result first.
    let classification = selector
        .checked_sub(engine_decisions::ctargeted_decision::SELECTOR_VIEW)
        .ok_or("targeted execution selector underflow")
        .and_then(|instance| {
            targeted_execution(&memory, BASE.load(Ordering::Acquire), instance, definition)
        });
    match classification {
        Ok(source) => decision(source, definition, country, Some(selector)),
        Err(reason) => feed::refuse(reason),
    }
    type Original = unsafe extern "system-unwind" fn(usize, usize, usize) -> usize;
    let original: Original =
        unsafe { std::mem::transmute(DECISION_ORIGINAL[4].load(Ordering::Acquire)) };
    let result = timing.original(|| unsafe { original(definition, country, selector) });
    flight.returned();
    result
}
fn targeted_execution(
    memory: &impl Memory,
    base: usize,
    instance: usize,
    definition: usize,
) -> Result<EventFeedSource, &'static str> {
    if engine_native::read::<u64>(memory, instance)
        != base
            .checked_add(engine_decisions::ctargeted_decision::VTABLE_RVA)
            .map(|value| value as u64)
        || field::<u64>(
            memory,
            instance,
            engine_decisions::ctargeted_decision::DEFINITION,
        )? as usize
            != definition
    {
        return Err("targeted execution instance identity differs");
    }
    match field::<i32>(
        memory,
        instance,
        engine_decisions::ctargeted_decision::STATE,
    )? {
        0 => Ok(EventFeedSource::DecisionSelected),
        1 | 3 => Ok(EventFeedSource::DecisionCompletionEffect),
        _ => Err("targeted execution lifecycle result is unsupported"),
    }
}

fn last_instance(
    memory: &impl Memory,
    status: usize,
    container: usize,
) -> Result<usize, &'static str> {
    let address = status
        .checked_add(container)
        .ok_or("mission container address overflow")?;
    let array = engine_native::cpdx_array::Array::read(memory, address, 8)
        .ok_or("mission append header unreadable")?;
    let last = array
        .count
        .checked_sub(1)
        .and_then(|n| array.element(n, 8))
        .ok_or("mission append has no selected entry")?;
    field::<u64>(memory, last, 0).map(|pointer| pointer as usize)
}
fn activation(status: usize, definition: usize, instance: Option<usize>, days: i32) {
    let memory = unsafe { engine_native::memory::LiveMemory::new() };
    let base = BASE.load(Ordering::Acquire);
    let result = (|| {
        let instance = match instance {
            Some(instance) => instance,
            None => last_instance(
                &memory,
                status,
                engine_decisions::cdecision_status::ACTIVE_TIMED,
            )?,
        };
        if instance == 0
            || engine_native::read::<u64>(&memory, instance)
                != base
                    .checked_add(engine_decisions::ctimed_decision::VTABLE_RVA)
                    .map(|at| at as u64)
            || field::<u64>(
                &memory,
                instance,
                engine_decisions::ctimed_decision::DEFINITION,
            )? as usize
                != definition
        {
            return Err("mission activation selected instance identity differs");
        }
        if !feed::decisions::activated(status, instance, 3, 0, days) {
            decision_result(
                EventFeedSource::DecisionActivated,
                definition,
                lifecycle::owner(status)?,
                None,
                Some(0),
                Some(days),
            );
        }
        Ok(())
    })();
    if let Err(reason) = result {
        feed::refuse(reason);
    }
}
unsafe extern "system-unwind" fn activate_targeted(
    status: usize,
    definition: usize,
    selector: usize,
) -> usize {
    let mut flight = Flight::enter();
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::EventFeedProvenance);
    type Original = unsafe extern "system-unwind" fn(usize, usize, usize) -> usize;
    let original: Original =
        unsafe { std::mem::transmute(ACTIVATION_ORIGINAL.load(Ordering::Acquire)) };
    let result = timing.original(|| unsafe { original(status, definition, selector) });
    let memory = unsafe { engine_native::memory::LiveMemory::new() };
    let observed = (|| {
        let container = if field::<u8>(
            &memory,
            definition,
            engine_decisions::cdecision::TARGETED_TIMED,
        )? != 0
        {
            engine_decisions::cdecision_status::TARGETED_TIMED
        } else {
            engine_decisions::cdecision_status::TARGETED
        };
        let instance = last_instance(&memory, status, container)?;
        let view = instance
            .checked_add(engine_decisions::ctargeted_decision::SELECTOR_VIEW)
            .ok_or("targeted activation selector overflow")?;
        let state = field(
            &memory,
            instance,
            engine_decisions::ctargeted_decision::STATE,
        )?;
        let days = field(
            &memory,
            instance,
            engine_decisions::ctargeted_decision::DAYS,
        )?;
        if !feed::decisions::activated(
            status,
            instance,
            if container == engine_decisions::cdecision_status::TARGETED {
                4
            } else {
                5
            },
            state,
            days,
        ) {
            decision_result(
                EventFeedSource::DecisionActivated,
                definition,
                lifecycle::owner(status)?,
                Some(view),
                Some(state),
                Some(days),
            );
        }
        Ok::<_, &'static str>(())
    })();
    if let Err(reason) = observed {
        feed::refuse(reason);
    }
    flight.returned();
    result
}

fn field<T: engine_native::memory::Raw>(
    memory: &impl Memory,
    object: usize,
    offset: usize,
) -> Result<T, &'static str> {
    object
        .checked_add(offset)
        .and_then(|at| engine_native::read(memory, at))
        .ok_or("script event field unreadable or overflowed")
}
pub(crate) fn definition(
    memory: &impl Memory,
    base: usize,
    event: usize,
) -> Result<
    (
        replay_data::event_feed_rows::EventFeedEventIdentity,
        Descriptor,
    ),
    &'static str,
> {
    if event == 0
        || engine_native::read::<u64>(memory, event)
            != base.checked_add(cevent::VTABLE_RVA).map(|at| at as u64)
    {
        return Err("script event definition vtable unreadable or differs");
    }
    let reference = field::<[u8; 8]>(memory, event, cevent::REFERENCE)?;
    let id = replay_data::event_feed_rows::EventFeedEventIdentity {
        kind: u32::from_le_bytes(reference[..4].try_into().unwrap()),
        id: u32::from_le_bytes(reference[4..].try_into().unwrap()),
    };
    let token = Descriptor::read(
        memory,
        event
            .checked_add(cevent::TOKEN)
            .ok_or("script event token address overflow")?,
    )?;
    Ok((id, token))
}

unsafe extern "system-unwind" fn tap(registers: &Registers, index: usize) {
    let _timing = crate::hook_timing::enter(crate::hook_timing::Family::EventFeedProvenance);
    match index {
        0 => {
            feed::record_script(|world, memory, base| {
                let event = registers.rsi;
                let (id, token) = definition(memory, base, event)?;
                let actor =
                    field::<u64>(memory, registers.rbp, native::DISPATCH_ACTOR_HOME)? as usize;
                let scope =
                    field::<u64>(memory, registers.rbp, native::DISPATCH_SCOPE_HOME)? as usize;
                let news = field::<u8>(memory, event, cevent::NEWS_CHROME)? != 0;
                Ok(Some(ScriptFact {
                    source: if news {
                        EventFeedSource::NewsEvent
                    } else {
                        EventFeedSource::ScriptEvent
                    },
                    actor: world.country_at(actor)?,
                    target: world.country_at(registers.r13)?,
                    cause: crate::war_provenance::inside(),
                    token,
                    scope: Some(scope),
                    dispatch_frame: registers.rbp,
                    dispatch_definition: event,
                    script: EventFeedScript {
                        native_event: Some(id),
                        ..Default::default()
                    },
                }))
            });
        }
        1 => {
            let memory = unsafe { engine_native::memory::LiveMemory::new() };
            match field::<u32>(&memory, registers.rsi, cevent::OPTION_COUNT) {
                Ok(0) => {} // Optionless dispatch did not mint an instance.
                Ok(_) => feed::script_instance(registers.rbp, registers.rdi as u32),
                Err(reason) => feed::refuse(reason),
            }
        }
        2 | 3 => option(registers.rbx, registers.rsi, registers.rdi as i32),
        4 | 5 => activation(
            registers.rbp,
            registers.rsi,
            (index == 5).then_some(registers.rdi),
            registers.r12 as i32,
        ),
        6 | 7 => activation(
            registers.r15,
            registers.r14,
            (index == 7).then_some(registers.rdi),
            registers.r12 as i32,
        ),
        8 => feed::record_script(|world, memory, base| {
            if world.state_pointer() != registers.rsi {
                return Err("fired-event erasure world differs");
            }
            let mut fact =
                crate::event_feed_history::registry::fired(world, memory, base, registers.rdi)?;
            fact.script.change =
                Some(replay_data::event_feed_rows::EventFeedScriptChange::FiredRemoved);
            Ok(Some(fact))
        }),
        _ => feed::refuse("script event tap identity differs"),
    }
}

unsafe extern "system-unwind" fn register(
    state: usize,
    event: usize,
    actor: usize,
    scope: usize,
    date: usize,
    requested: usize,
) -> usize {
    let mut flight = Flight::enter();
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::EventFeedProvenance);
    type Original =
        unsafe extern "system-unwind" fn(usize, usize, usize, usize, usize, usize) -> usize;
    let original: Original =
        unsafe { std::mem::transmute(REGISTER_ORIGINAL.load(Ordering::Acquire)) };
    let result =
        timing.original(|| unsafe { original(state, event, actor, scope, date, requested) });
    feed::registered_script_instance(state, event, scope, result as u32);
    flight.returned();
    result
}

unsafe extern "system-unwind" fn remove_pending(state: usize, instance: usize) -> usize {
    let mut flight = Flight::enter();
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::EventFeedProvenance);
    let memory = unsafe { engine_native::memory::LiveMemory::new() };
    let before = field::<u32>(
        &memory,
        state,
        engine_native::cgame_state::PENDING_EVENTS_COUNT,
    );
    type Original = unsafe extern "system-unwind" fn(usize, usize) -> usize;
    let original: Original =
        unsafe { std::mem::transmute(REGISTRY_ORIGINAL[0].load(Ordering::Acquire)) };
    let result = timing.original(|| unsafe { original(state, instance) });
    feed::record_script(|world, memory, _| {
        if world.state_pointer() != state {
            return Err("pending event removal world differs");
        }
        let before = before?;
        let after = field::<u32>(
            memory,
            state,
            engine_native::cgame_state::PENDING_EVENTS_COUNT,
        )?;
        if before == after {
            return Ok(None);
        } // Native no-match call.
        if after.checked_add(1) != Some(before) {
            return Err("pending event removal count differs");
        }
        crate::event_feed_history::registry::marker(
            world,
            memory,
            replay_data::event_feed_rows::EventFeedScriptChange::PendingRemoved,
            Some(instance as u32),
        )
        .map(Some)
    });
    flight.returned();
    result
}

unsafe extern "system-unwind" fn retire_pending(array: usize, ordinal: usize) -> usize {
    let mut flight = Flight::enter();
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::EventFeedProvenance);
    let memory = unsafe { engine_native::memory::LiveMemory::new() };
    let before = engine_native::cpdx_array::Array::read(
        &memory,
        array,
        engine_events::script_event_registry::PENDING_STRIDE,
    )
    .ok_or("pending retirement header unreadable or invalid")
    .and_then(|header| {
        let entry = header
            .element(
                ordinal as u32,
                engine_events::script_event_registry::PENDING_STRIDE,
            )
            .ok_or("pending retirement ordinal outside array")?;
        Ok((
            header.count,
            field::<u32>(
                &memory,
                entry,
                engine_events::script_event_registry::PENDING_INSTANCE,
            )?,
        ))
    });
    type Original = unsafe extern "system-unwind" fn(usize, usize) -> usize;
    let original: Original =
        unsafe { std::mem::transmute(REGISTRY_ORIGINAL[1].load(Ordering::Acquire)) };
    let result = timing.original(|| unsafe { original(array, ordinal) });
    feed::record_script(|world, memory, _| {
        if world
            .state_pointer()
            .checked_add(engine_native::cgame_state::PENDING_EVENTS)
            != Some(array)
        {
            return Err("pending retirement world differs");
        }
        let (count, instance) = before?;
        let after = engine_native::cpdx_array::Array::read(
            memory,
            array,
            engine_events::script_event_registry::PENDING_STRIDE,
        )
        .ok_or("pending retirement result unreadable")?;
        if after.count.checked_add(1) != Some(count) {
            return Err("pending retirement count differs");
        }
        crate::event_feed_history::registry::marker(
            world,
            memory,
            replay_data::event_feed_rows::EventFeedScriptChange::PendingRemoved,
            Some(instance),
        )
        .map(Some)
    });
    flight.returned();
    result
}

unsafe extern "system-unwind" fn insert_fired(set: usize, definition: usize) -> usize {
    let mut flight = Flight::enter();
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::EventFeedProvenance);
    type Original = unsafe extern "system-unwind" fn(usize, usize) -> usize;
    let original: Original =
        unsafe { std::mem::transmute(REGISTRY_ORIGINAL[2].load(Ordering::Acquire)) };
    let result = timing.original(|| unsafe { original(set, definition) });
    feed::record_script(|world, memory, base| {
        if world
            .state_pointer()
            .checked_add(engine_native::cgame_state::FIRED_EVENTS)
            != Some(set)
        {
            return Err("fired-event insertion world differs");
        }
        crate::event_feed_history::registry::fired(world, memory, base, definition).map(Some)
    });
    flight.returned();
    result
}

fn option(event: usize, scope: usize, ordinal: i32) {
    feed::record_script(|world, memory, base| {
        let (id, token) = definition(memory, base, event)?;
        // This execution entry has one caller: the select command's body.
        // Authenticate its owned scope before taking the instance/actor words.
        let command = scope
            .checked_sub(native::OPTION_COMMAND_SCOPE)
            .ok_or("event option command address underflow")?;
        if engine_native::read::<u64>(memory, command)
            != base
                .checked_add(native::OPTION_COMMAND_VTABLE_RVA)
                .map(|at| at as u64)
        {
            return Err("event option command vtable unreadable or differs");
        }
        let actor = command
            .checked_add(native::OPTION_COMMAND_ACTOR)
            .ok_or("event option actor address overflow")?;
        Ok(Some(ScriptFact {
            source: EventFeedSource::EventOption,
            actor: world.country_at(actor)?,
            target: world.country(field::<u32>(
                memory,
                scope,
                engine_events::cevent_scope::COUNTRY_TAG,
            )?)?,
            cause: crate::war_provenance::inside(),
            token,
            scope: Some(scope),
            dispatch_frame: 0,
            dispatch_definition: 0,
            script: EventFeedScript {
                native_event: Some(id),
                instance: Some(field(memory, command, native::OPTION_COMMAND_INSTANCE)?),
                option: Some(ordinal),
                ..Default::default()
            },
        }))
    });
}

fn hook_failure(site: native::Site, stage: &str, cause: impl std::fmt::Display) -> &'static str {
    terminal_hook_failure(site.label(), stage, cause)
}

pub(super) fn terminal_hook_failure(
    label: &str,
    stage: &str,
    cause: impl std::fmt::Display,
) -> &'static str {
    // A terminal installation failure is retained for status until unload,
    // matching the runtime's installation-message lifetime. Busy retries do
    // not enter this path or allocate a diagnostic.
    Box::leak(format!("{label}: {stage} failed: {cause}").into_boxed_str())
}

fn verify_hook(
    memory: &impl Memory,
    base: usize,
    site: native::Site,
    len: usize,
) -> Result<usize, &'static str> {
    let at = base
        .checked_add(site.rva)
        .ok_or_else(|| hook_failure(site, "sentinel", "address overflow"))?;
    let mut actual = [0_u8; 16];
    if !memory.copy(at, &mut actual[..len]) {
        return Err(hook_failure(site, "sentinel", "native bytes unreadable"));
    }
    if actual[..len] != site.sentinel[..len] {
        return Err(hook_failure(
            site,
            "sentinel",
            format_args!(
                "expected {:02x?}, read {:02x?}",
                &site.sentinel[..len],
                &actual[..len]
            ),
        ));
    }
    Ok(at)
}

/// # Safety
/// The host owns publication, restoration, native-flight and executable-range
/// quiescence with the complete event-feed family.
pub unsafe fn hooks(
    memory: &impl Memory,
    base: usize,
) -> Result<(Vec<Hook>, Vec<Tap>), &'static str> {
    let verify = |site: native::Site| verify_hook(memory, base, site, site.sentinel.len());
    let mut taps = Vec::new();
    for (index, site) in native::TAPS.into_iter().enumerate() {
        let at = verify(site)?;
        taps.push(
            unsafe { Tap::new(at, tap, index, site.sentinel) }
                .map_err(|cause| hook_failure(site, "construction", cause))?
                .named(site.label()),
        );
    }
    let mut hooks = Vec::new();
    for (site, callback, original) in [
        (
            native::REGISTER,
            register as *const () as usize,
            &REGISTER_ORIGINAL,
        ),
        (
            native::ACTIVATE_TARGETED,
            activate_targeted as *const () as usize,
            &ACTIVATION_ORIGINAL,
        ),
        (
            native::REMOVE_PENDING,
            remove_pending as *const () as usize,
            &REGISTRY_ORIGINAL[0],
        ),
        (
            native::RETIRE_PENDING,
            retire_pending as *const () as usize,
            &REGISTRY_ORIGINAL[1],
        ),
        (
            native::INSERT_FIRED,
            insert_fired as *const () as usize,
            &REGISTRY_ORIGINAL[2],
        ),
    ] {
        let hook = unsafe { Hook::new(verify(site)?, callback, site.sentinel) }
            .map_err(|cause| hook_failure(site, "construction", cause))?
            .named(site.label());
        original.store(hook.trampoline(), Ordering::Release);
        hooks.push(hook);
    }
    for (index, ((name, rva, sentinel), callback)) in
        engine_decisions::cdecision::EVENT_FEED_ENTRIES
            .into_iter()
            .zip([
                cancel as *const () as usize,
                remove as *const () as usize,
                targeted_cancel as *const () as usize,
                targeted_remove as *const () as usize,
                targeted_execute as *const () as usize,
            ])
            .enumerate()
    {
        let site = native::Site {
            name,
            rva,
            sentinel,
        };
        let hook = unsafe { Hook::new(verify(site)?, callback, sentinel) }
            .map_err(|cause| hook_failure(site, "construction", cause))?
            .named(site.label());
        DECISION_ORIGINAL[index].store(hook.trampoline(), Ordering::Release);
        hooks.push(hook);
    }
    let (delayed_hooks, delayed_taps) = unsafe { delayed::hooks(memory, base) }?;
    hooks.extend(delayed_hooks);
    taps.extend(delayed_taps);
    let (lifecycle_hooks, lifecycle_taps) = unsafe { lifecycle::hooks(memory, base) }?;
    hooks.extend(lifecycle_hooks);
    taps.extend(lifecycle_taps);
    let (decision_hooks, decision_taps) = unsafe { decisions::hooks(memory, base) }?;
    hooks.extend(decision_hooks);
    taps.extend(decision_taps);
    BASE.store(base, Ordering::Release);
    Ok((hooks, taps))
}

/// The script-variable family owns the delayed pass and forwards it here.
pub fn delayed_pass(country: usize, run: impl FnOnce() -> usize) -> usize {
    delayed::forwarded_pass(country, run)
}
/// # Safety
/// The script-variable owner publishes/restores these selected cursor taps
/// together with the pass entry and retains their ranges through quiescence.
pub unsafe fn delayed_pass_taps(
    memory: &impl Memory,
    base: usize,
) -> Result<Vec<Tap>, &'static str> {
    unsafe { delayed::forwarded_taps(memory, base) }
}

/// # Safety
/// The politics family owns this function and forwards its selected final
/// store through this tap, publishing/restoring it with its entry detour.
pub unsafe fn targeted_selection_tap(
    memory: &impl Memory,
    base: usize,
) -> Result<Tap, &'static str> {
    unsafe { decisions::selection_tap(memory, base) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn adjacent_calls_verify_only_their_owned_prefix_and_name_failures() {
        for site in [
            native::decisions::REACTIVATE_TARGETED_DAYS,
            native::decisions::REQUEST_TARGETED_TIMED,
        ] {
            let len = native::decisions::call_sentinel_len(site);
            let memory = engine_native::memory::SliceMemory {
                base: site.rva,
                bytes: &site.sentinel[..len],
            };
            assert_eq!(verify_hook(&memory, 0, site, len), Ok(site.rva));
            let error = verify_hook(&memory, 0, site, 16).unwrap_err();
            assert!(error.contains(site.name));
            assert!(error.contains("sentinel failed: native bytes unreadable"));
            let mut bytes = site.sentinel;
            bytes[0] ^= 1;
            let memory = engine_native::memory::SliceMemory {
                base: site.rva,
                bytes: &bytes,
            };
            let error = verify_hook(&memory, 0, site, len).unwrap_err();
            assert!(error.contains("sentinel failed: expected"));
            assert!(error.contains("read"));
            assert!(
                hook_failure(site, "construction", "synthetic relay refusal")
                    .ends_with("construction failed: synthetic relay refusal")
            );
            assert!(
                hook_failure(site, "publication", "synthetic byte verification failure")
                    .ends_with("publication failed: synthetic byte verification failure")
            );
        }
    }
    #[test]
    fn targeted_execution_classifies_the_selected_instance_result_without_enclosing_provenance() {
        let mut bytes = [0_u8; 0x38];
        bytes[..8].copy_from_slice(
            &(engine_decisions::ctargeted_decision::VTABLE_RVA as u64).to_le_bytes(),
        );
        bytes[0x10..0x18].copy_from_slice(&99_u64.to_le_bytes());
        for (state, expected) in [
            (0_i32, Some(EventFeedSource::DecisionSelected)),
            (1, Some(EventFeedSource::DecisionCompletionEffect)),
            (3, Some(EventFeedSource::DecisionCompletionEffect)),
            (2, None),
            (-1, None),
        ] {
            bytes[0x28..0x2c].copy_from_slice(&state.to_le_bytes());
            let memory = engine_native::memory::SliceMemory {
                base: 0x1000,
                bytes: &bytes,
            };
            assert_eq!(targeted_execution(&memory, 0, 0x1000, 99).ok(), expected);
            assert!(targeted_execution(&memory, 0, 0x1000, 100).is_err());
        }
    }
}
