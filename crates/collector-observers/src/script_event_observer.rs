
use crate::{
    event_feed_history::ScriptFact,
    event_feed_observer as feed,
    hook::{Hook, Registers, Tap},
};
use engine_decisions::cevent;
use engine_events::script_event_sites as native;
use engine_native_memory::{Memory, msvc_string::Descriptor};
use replay_data::event_feed_rows::{EventFeedScript, EventFeedSource};
use std::sync::atomic::{AtomicUsize, Ordering};
mod decisions;
mod delayed;
mod lifecycle;
pub mod staging;

static BASE: AtomicUsize = AtomicUsize::new(0);
static REGISTER_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static REGISTRY_ORIGINAL: [AtomicUsize; 3] = [const { AtomicUsize::new(0) }; 3];
static FLIGHT_GATE: collector_primitives::asymmetric::FlightGate =
    collector_primitives::asymmetric::FlightGate::new(0);
static DECISION_ORIGINAL: [AtomicUsize; 5] = [const { AtomicUsize::new(0) }; 5];
static ACTIVATION_ORIGINAL: AtomicUsize = AtomicUsize::new(0);

pub(crate) struct Flight {
    guard: Option<collector_primitives::asymmetric::FlightGuard<'static>>,
    normal: bool,
}
impl Flight {
    #[inline(always)]
    pub(crate) fn enter() -> Self {
        Self {
            guard: Some(FLIGHT_GATE.enter(true)),
            normal: false,
        }
    }
    #[inline(always)]
    pub(crate) fn returned(&mut self) {
        self.normal = true;
    }
}
impl Drop for Flight {
    #[inline(always)]
    fn drop(&mut self) {
        if let Some(guard) = self.guard.take() {
            if self.normal {
                drop(guard);
            } else {
                guard.leak_and_retain();
            }
        }
    }
}
pub fn active() -> usize {
    FLIGHT_GATE.active()
}
pub(crate) fn quiet_epoch() -> Option<u64> {
    FLIGHT_GATE.quiet_epoch()
}

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
            || engine_sites::at(base, engine_decisions::cdecision::VTABLE_RVA).is_none_or(
                |vtable| engine_native_memory::read::<u64>(memory, definition) != Some(vtable as u64),
            )
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
            selected_instance(memory, base, selector, definition)?;
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
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    match targeted_execution(&memory, BASE.load(Ordering::Acquire), selector, definition) {
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
fn selected_instance(
    memory: &impl Memory,
    base: usize,
    selector: usize,
    definition: usize,
) -> Result<Option<usize>, &'static str> {
    use engine_decisions::ctargeted_decision as targeted;
    let vtable = |key| engine_sites::at(base, key).ok_or("targeted decision vtable withheld");
    if let Some(instance) = selector.checked_sub(targeted::SELECTOR_VIEW)
        && engine_native_memory::read::<u64>(memory, instance)
            == Some(vtable(targeted::VTABLE_RVA)? as u64)
    {
        return if field::<u64>(memory, instance, targeted::DEFINITION)? as usize == definition {
            Ok(Some(instance))
        } else {
            Err("targeted decision selector instance definition differs")
        };
    }
    if engine_native_memory::read::<u64>(memory, selector) == Some(vtable(targeted::TARGET_VTABLE_RVA)? as u64)
    {
        Ok(None)
    } else {
        Err("targeted decision selector is neither an instance view nor a target")
    }
}
fn targeted_execution(
    memory: &impl Memory,
    base: usize,
    selector: usize,
    definition: usize,
) -> Result<EventFeedSource, &'static str> {
    let Some(instance) = selected_instance(memory, base, selector, definition)? else {
        return Ok(EventFeedSource::DecisionSelected);
    };
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
    let array = engine_native_memory::cpdx_array::Array::read(memory, address, 8)
        .ok_or("mission append header unreadable")?;
    let last = array
        .count
        .checked_sub(1)
        .and_then(|n| array.element(n, 8))
        .ok_or("mission append has no selected entry")?;
    field::<u64>(memory, last, 0).map(|pointer| pointer as usize)
}
fn appended(status: usize, definition: usize, days: i32) {
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    let result = last_instance(
        &memory,
        status,
        engine_decisions::cdecision_status::ACTIVE_TIMED,
    )
    .and_then(|instance| publish_activation(&memory, status, definition, instance, 0, days));
    if let Err(reason) = result {
        feed::refuse(reason);
    }
}
pub(super) fn reactivated(status: usize, definition: usize, instance: usize) {
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    let result = (|| {
        let days = field::<i32>(&memory, instance, engine_decisions::ctimed_decision::DAYS)?;
        let state = field::<i32>(&memory, instance, engine_decisions::ctimed_decision::STATE)?;
        publish_activation(&memory, status, definition, instance, state, days)
    })();
    if let Err(reason) = result {
        feed::refuse(reason);
    }
}
fn publish_activation(
    memory: &impl Memory,
    status: usize,
    definition: usize,
    instance: usize,
    state: i32,
    days: i32,
) -> Result<(), &'static str> {
    let base = BASE.load(Ordering::Acquire);
    if instance == 0
        || engine_sites::at(base, engine_decisions::ctimed_decision::VTABLE_RVA).is_none_or(
            |vtable| engine_native_memory::read::<u64>(memory, instance) != Some(vtable as u64),
        )
        || field::<u64>(
            memory,
            instance,
            engine_decisions::ctimed_decision::DEFINITION,
        )? as usize
            != definition
    {
        return Err("mission activation selected instance identity differs");
    }
    #[cfg(test)]
    staging::published(staging::Published::Activation {
        instance,
        state,
        days,
    });
    if !feed::decisions::activated(status, instance, 3, state, days) {
        decision_result(
            EventFeedSource::DecisionActivated,
            definition,
            lifecycle::owner(status)?,
            None,
            Some(state),
            Some(days),
        );
    }
    Ok(())
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
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
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

fn field<T: engine_native_memory::memory::Raw>(
    memory: &impl Memory,
    object: usize,
    offset: usize,
) -> Result<T, &'static str> {
    object
        .checked_add(offset)
        .and_then(|at| engine_native_memory::read(memory, at))
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
        || engine_sites::at(base, cevent::VTABLE_RVA)
            .is_none_or(|vtable| engine_native_memory::read::<u64>(memory, event) != Some(vtable as u64))
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
            let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
            match field::<u32>(&memory, registers.rsi, cevent::OPTION_COUNT) {
                Ok(0) => {}
                Ok(_) => feed::script_instance(registers.rbp, registers.rdi as u32),
                Err(reason) => feed::refuse(reason),
            }
        }
        2 | 3 => option(registers.rbx, registers.rsi, registers.rdi as i32),
        4 => appended(registers.rbp, registers.rsi, registers.r12 as i32),
        6 => appended(registers.r15, registers.r14, registers.r12 as i32),
        5 => staging::stage(
            staging::Unit::AutoActivation,
            staging::Witness::Reactivated {
                status: registers.rbp,
                definition: registers.rsi,
                instance: registers.rdi,
            },
        ),
        7 => staging::stage(
            staging::Unit::ScriptedActivation,
            staging::Witness::Reactivated {
                status: registers.r15,
                definition: registers.r14,
                instance: registers.rdi,
            },
        ),
        8 => staging::stage(
            staging::Unit::FiredRetirement,
            staging::Witness::FiredMatch {
                state: registers.rsi,
                definition: registers.rdi,
            },
        ),
        _ => feed::refuse("script event tap identity differs"),
    }
}
pub(super) fn retired_fired(state: usize, definition: usize) {
    #[cfg(test)]
    staging::published(staging::Published::Fired { definition });
    feed::record_script(|world, memory, base| {
        if world.state_pointer() != state {
            return Err("fired-event erasure world differs");
        }
        let mut fact = crate::event_feed_history::registry::fired(world, memory, base, definition)?;
        fact.script.change =
            Some(replay_data::event_feed_rows::EventFeedScriptChange::FiredRemoved);
        Ok(Some(fact))
    });
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
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
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
        }
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
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    let before = engine_native_memory::cpdx_array::Array::read(
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
        let after = engine_native_memory::cpdx_array::Array::read(
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
        let command = scope
            .checked_sub(native::OPTION_COMMAND_SCOPE)
            .ok_or("event option command address underflow")?;
        if engine_sites::at(base, native::OPTION_COMMAND_VTABLE_RVA)
            .is_none_or(|vtable| engine_native_memory::read::<u64>(memory, command) != Some(vtable as u64))
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

fn hook_failure(site: native::Site, stage: &str, cause: impl std::fmt::Display) -> String {
    terminal_hook_failure(site.label(), stage, cause)
}

pub(super) fn terminal_hook_failure(
    label: &str,
    stage: &str,
    cause: impl std::fmt::Display,
) -> String {
    format!("{label}: {stage} failed: {cause}")
}

#[doc = " The site's address and the 16 bytes its patch replaces, of which the first `N` are verified"]
#[doc = " and owned."]
fn verify_hook<const N: usize>(
    base: usize,
    site: native::Site,
) -> Result<(usize, [u8; 16]), String> {
    let withheld = |cause| hook_failure(site, "sentinel", cause);
    let code = engine_sites::code(base, site.rva).map_err(withheld)?;
    let owned = code.original::<N>().map_err(withheld)?;
    let mut original = [0; 16];
    original[..N].copy_from_slice(&owned);
    Ok((code.address, original))
}

/// # Safety
/// The host owns publication, restoration, native-flight and executable-range
/// quiescence with the complete event-feed family.
pub unsafe fn hooks(
    _memory: &impl Memory,
    base: usize,
) -> Result<(Vec<Hook>, Vec<Tap>), String> {
    let verify = |site: native::Site| verify_hook::<16>(base, site);
    let mut taps = Vec::new();
    for (index, site) in native::TAPS.into_iter().enumerate() {
        let (at, original) = verify(site)?;
        taps.push(
            unsafe { Tap::new(at, tap, index, original) }
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
        let (at, original_bytes) = verify(site)?;
        let hook = unsafe { Hook::new(at, callback, original_bytes) }
            .map_err(|cause| hook_failure(site, "construction", cause))?
            .named(site.label());
        original.store(hook.trampoline(), Ordering::Release);
        hooks.push(hook);
    }
    for (index, ((name, rva), callback)) in engine_decisions::cdecision::EVENT_FEED_ENTRIES
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
        let site = native::Site { name, rva };
        let (at, original) = verify(site)?;
        let hook = unsafe { Hook::new(at, callback, original) }
            .map_err(|cause| hook_failure(site, "construction", cause))?
            .named(site.label());
        DECISION_ORIGINAL[index].store(hook.trampoline(), Ordering::Release);
        hooks.push(hook);
    }
    let (delayed_hooks, delayed_taps) = unsafe { delayed::hooks(base) }?;
    hooks.extend(delayed_hooks);
    taps.extend(delayed_taps);
    let (lifecycle_hooks, lifecycle_taps) = unsafe { lifecycle::hooks(base) }?;
    hooks.extend(lifecycle_hooks);
    taps.extend(lifecycle_taps);
    let (decision_hooks, decision_taps) = unsafe { decisions::hooks(base) }?;
    hooks.extend(decision_hooks);
    taps.extend(decision_taps);
    BASE.store(base, Ordering::Release);
    Ok((hooks, taps))
}

pub fn delayed_pass(country: usize, run: impl FnOnce() -> usize) -> usize {
    delayed::forwarded_pass(country, run)
}
/// # Safety
/// The script-variable owner publishes/restores these selected cursor taps
/// together with the pass entry and retains their ranges through quiescence.
pub unsafe fn delayed_pass_taps(
    _memory: &impl Memory,
    base: usize,
) -> Result<Vec<Tap>, String> {
    unsafe { delayed::forwarded_taps(base) }
}

pub fn targeted_selection(run: impl FnOnce() -> usize) -> usize {
    let detour = staging::Detour::enter(staging::Unit::TargetedSelection);
    let result = run();
    #[cfg(any(test, feature = "test-support"))]
    SELECTION_CAUSE.set(Some(crate::war_provenance::inside()));
    detour.publish(|witness| match witness {
        staging::Witness::Selected { instance } => decisions::selected_result(instance, true),
        _ => feed::refuse(staging::OTHER_FRAME),
    });
    result
}

#[cfg(any(test, feature = "test-support"))]
std::thread_local! {
    static SELECTION_CAUSE: std::cell::Cell<Option<replay_data::event_feed_rows::EventFeedCause>> =
        const { std::cell::Cell::new(None) };
}
#[cfg(any(test, feature = "test-support"))]
pub fn take_selection_cause() -> Option<replay_data::event_feed_rows::EventFeedCause> {
    SELECTION_CAUSE.take()
}

/// # Safety
/// The politics family owns this function and forwards its selected final
/// store through this tap, which only stages its witness for
/// [`targeted_selection`].
pub unsafe fn targeted_selection_tap(
    _memory: &impl Memory,
    base: usize,
) -> Result<Tap, String> {
    unsafe { decisions::selection_tap(base) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn targeted_execution_classifies_the_selected_instance_result_without_enclosing_provenance() {
        let _globals = crate::event_feed_observer::test_globals();
        use engine_decisions::ctargeted_decision::{TARGET_VTABLE_RVA, VTABLE_RVA};
        let mut bytes = [0_u8; 0x38];
        bytes[..8].copy_from_slice(&(VTABLE_RVA.0 as u64).to_le_bytes());
        bytes[0x10..0x18].copy_from_slice(&99_u64.to_le_bytes());
        bytes[0x18..0x20].copy_from_slice(&(TARGET_VTABLE_RVA.0 as u64).to_le_bytes());
        for (state, expected) in [
            (0_i32, Some(EventFeedSource::DecisionSelected)),
            (1, Some(EventFeedSource::DecisionCompletionEffect)),
            (3, Some(EventFeedSource::DecisionCompletionEffect)),
            (2, None),
            (-1, None),
        ] {
            bytes[0x28..0x2c].copy_from_slice(&state.to_le_bytes());
            let memory = engine_native_memory::memory::SliceMemory {
                base: 0x1000,
                bytes: &bytes,
            };
            assert_eq!(targeted_execution(&memory, 0, 0x1018, 99).ok(), expected);
            assert_eq!(
                targeted_execution(&memory, 0, 0x1018, 100),
                Err("targeted decision selector instance definition differs")
            );
        }
        let mut standalone = [0_u8; 0x28];
        standalone[0x18..0x20].copy_from_slice(&(TARGET_VTABLE_RVA.0 as u64).to_le_bytes());
        standalone[0x20..0x24].copy_from_slice(&7_i32.to_le_bytes());
        let memory = engine_native_memory::memory::SliceMemory {
            base: 0x1000,
            bytes: &standalone,
        };
        assert_eq!(
            targeted_execution(&memory, 0, 0x1018, 99),
            Ok(EventFeedSource::DecisionSelected)
        );
        assert_eq!(selected_instance(&memory, 0, 0x1018, 99), Ok(None));
        standalone[0x18..0x20].fill(0);
        let memory = engine_native_memory::memory::SliceMemory {
            base: 0x1000,
            bytes: &standalone,
        };
        assert_eq!(
            targeted_execution(&memory, 0, 0x1018, 99),
            Err("targeted decision selector is neither an instance view nor a target")
        );
    }
}
