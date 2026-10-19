//! Sparse scalar duration writers reuse the native selected instance and owner.
use super::*;

fn values(
    memory: &impl Memory,
    base: usize,
    instance: usize,
    cooldown: bool,
    delta: Option<i32>,
) -> Result<(usize, i32), &'static str> {
    let (definition, days) = if cooldown {
        cooldown_result(memory, base, instance)?
    } else {
        let (definition, _) = selected(memory, base, instance, false)?;
        (
            definition,
            field(memory, instance, engine_decisions::ctimed_decision::DAYS)?,
        )
    };
    Ok((
        definition,
        delta.map_or(days, |delta| days.wrapping_add(delta)),
    ))
}
fn duration_script(days: i32) -> EventFeedScript {
    // These writers store only duration, never a lifecycle result.
    EventFeedScript {
        days: Some(days),
        ..Default::default()
    }
}
fn emit(instance: usize, tag: usize, cooldown: bool, delta: Option<i32>) {
    let memory = unsafe { engine_native::memory::LiveMemory::new() };
    if let Ok((_, days)) = values(
        &memory,
        BASE.load(Ordering::Acquire),
        instance,
        cooldown,
        delta,
    ) && feed::decisions::write_result(
        instance,
        None,
        Some(days),
        EventFeedSource::DecisionDurationChanged,
    ) {
        return;
    }
    feed::record_script(|world, memory, base| {
        let (definition, days) = values(memory, base, instance, cooldown, delta)?;
        if engine_native::read::<u64>(memory, definition)
            != base
                .checked_add(engine_decisions::cdecision::VTABLE_RVA)
                .map(|at| at as u64)
        {
            return Err("decision duration definition identity differs");
        }
        let token = Descriptor::read(
            memory,
            definition
                .checked_add(engine_decisions::cdecision::TOKEN)
                .ok_or("decision duration token overflow")?,
        )?;
        Ok(Some(ScriptFact {
            source: EventFeedSource::DecisionDurationChanged,
            actor: world.country_at(tag)?,
            target: world.country(0)?,
            token,
            scope: None,
            dispatch_frame: 0,
            dispatch_definition: 0,
            cause: crate::war_provenance::inside(),
            script: duration_script(days),
        }))
    });
}
pub(super) unsafe extern "system-unwind" fn tap(registers: &Registers, index: usize) {
    let _timing = crate::hook_timing::enter(crate::hook_timing::Family::EventFeedProvenance);
    let memory = unsafe { engine_native::memory::LiveMemory::new() };
    let operands = if index == 0 {
        Ok((registers.rdi, registers.r15, registers.rax as i32, false))
    } else {
        field::<i32>(&memory, registers.rbx, native::REMOVAL_EXTENSION_DAYS)
            .map(|delta| (registers.rax, registers.rsi, delta, true))
    };
    match operands.and_then(|(instance, scope, delta, cooldown)| {
        let tag = scope
            .checked_add(engine_events::cevent_scope::COUNTRY_TAG)
            .ok_or("decision duration scope overflow")?;
        Ok((instance, tag, delta, cooldown))
    }) {
        // Both authenticated call sites unconditionally forward this operand
        // to the selected scalar add. No trigger/effect runs before its store.
        Ok((instance, tag, delta, cooldown)) => emit(instance, tag, cooldown, Some(delta)),
        Err(reason) => feed::refuse(reason),
    }
}
pub(super) unsafe extern "system-unwind" fn display_duration(instance: usize, tag: usize) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::EventFeedProvenance);
    let mut flight = Flight::enter();
    type Original = unsafe extern "system-unwind" fn(usize, usize) -> usize;
    let original: Original = unsafe { std::mem::transmute(ORIGINAL[3].load(Ordering::Acquire)) };
    let result = timing.original(|| unsafe { original(instance, tag) });
    // The helper has completed every authored/default branch. Read its result,
    // including the default branch after effects changed its lifecycle.
    emit(instance, tag, false, None);
    flight.returned();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn extension_then_removal_preserves_intermediate_result_and_wrapping() {
        let mut bytes = [0; 0x40];
        bytes[..8]
            .copy_from_slice(&(engine_decisions::ctimed_decision::VTABLE_RVA as u64).to_le_bytes());
        bytes[engine_decisions::ctimed_decision::DEFINITION
            ..engine_decisions::ctimed_decision::DEFINITION + 8]
            .copy_from_slice(&99_u64.to_le_bytes());
        bytes[engine_decisions::ctimed_decision::DAYS..engine_decisions::ctimed_decision::DAYS + 4]
            .copy_from_slice(&10_i32.to_le_bytes());
        let memory = engine_native::memory::SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        assert_eq!(values(&memory, 0, 0x1000, false, Some(5)), Ok((99, 15)));
        assert_eq!(
            values(&memory, 0, 0x1000, false, Some(i32::MAX)),
            Ok((99, i32::MIN + 9))
        );
        assert!(values(&memory, 0, 0x1000, true, Some(5)).is_err());
    }
    #[test]
    fn removal_extension_uses_cooldown_layout_and_final_display_reads_only_actual_duration() {
        let mut bytes = [0; 0x40];
        bytes[..8].copy_from_slice(
            &(engine_decisions::cdecision_cooldown::VTABLE_RVA as u64).to_le_bytes(),
        );
        bytes[8..16].copy_from_slice(&88_u64.to_le_bytes());
        bytes[16..20].copy_from_slice(&10_i32.to_le_bytes());
        let memory = engine_native::memory::SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        assert_eq!(values(&memory, 0, 0x1000, true, Some(5)), Ok((88, 15)));
        bytes[..8]
            .copy_from_slice(&(engine_decisions::ctimed_decision::VTABLE_RVA as u64).to_le_bytes());
        bytes[16..24].copy_from_slice(&99_u64.to_le_bytes());
        bytes[24..28].copy_from_slice(&17_i32.to_le_bytes());
        for state in [0_i32, 1, 2, 3, 4] {
            bytes[28..32].copy_from_slice(&state.to_le_bytes());
            let memory = engine_native::memory::SliceMemory {
                base: 0x1000,
                bytes: &bytes,
            };
            assert_eq!(values(&memory, 0, 0x1000, false, None), Ok((99, 17)));
        }
        assert_eq!(duration_script(17).days, Some(17));
        assert_eq!(duration_script(17).lifecycle, None);
        assert!(
            native::LIFECYCLE_ENTRIES
                .iter()
                .any(|site| site.name == "MissionDisplayDuration")
        );
    }
}
