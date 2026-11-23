use super::*;
use super::{hook_failure, verify_hook};
pub(super) mod adjustments;
static ORIGINAL: [AtomicUsize; 4] = [const { AtomicUsize::new(0) }; 4];

pub(super) fn owner(status: usize) -> Result<usize, &'static str> {
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    if engine_sites::at(
        BASE.load(Ordering::Acquire),
        engine_decisions::cdecision_status::VTABLE_RVA,
    )
    .is_none_or(|vtable| engine_native_memory::read::<u64>(&memory, status) != Some(vtable as u64))
    {
        return Err("decision lifecycle status identity differs");
    }
    field::<u64>(&memory, status, engine_decisions::cdecision_status::COUNTRY)
        .map(|pointer| pointer as usize)
}
unsafe extern "system-unwind" fn tap(registers: &Registers, index: usize) {
    let _timing = crate::hook_timing::enter(crate::hook_timing::Family::EventFeedProvenance);
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    let result = (|| -> Result<(), &'static str> {
        if index == 0 || index == 6 {
            if index == 6 {
                let ordinal = registers
                    .r13
                    .wrapping_add(8)
                    .checked_div(8)
                    .and_then(|n| u32::try_from(n).ok())
                    .ok_or("taken decision erase ordinal overflow")?;
                feed::decisions::taken_erase(registers.rsi, registers.rdi, Some(ordinal), true);
            }
            decision(
                EventFeedSource::DecisionRemoved,
                if index == 0 {
                    registers.rbx
                } else {
                    registers.rdi
                },
                owner(if index == 0 {
                    registers.rdi
                } else {
                    registers.rsi
                })?,
                None,
            );
            Ok(())
        } else if index == 1 || index == 4 {
            field::<u64>(
                &memory,
                registers.rbx,
                engine_decisions::ctargeted_decision::DEFINITION,
            )
            .and_then(|definition| {
                feed::decisions::cut(registers.rbx);
                let days = field::<i32>(
                    &memory,
                    registers.rbx,
                    engine_decisions::ctargeted_decision::DAYS,
                )?;
                let combined = feed::decisions::write_result(
                    registers.rbx,
                    Some(0),
                    Some(days),
                    EventFeedSource::DecisionReenabled,
                );
                if !combined {
                    decision_result(
                        EventFeedSource::DecisionReenabled,
                        definition as usize,
                        owner(if index == 1 {
                            registers.rbp
                        } else {
                            registers.rsi
                        })?,
                        Some(registers.rbx + engine_decisions::ctargeted_decision::SELECTOR_VIEW),
                        Some(0),
                        Some(days),
                    );
                }
                Ok(())
            })
        } else if index == 5 || index == 7 {
            let (definition, days) =
                cooldown_result(&memory, BASE.load(Ordering::Acquire), registers.r8)?;
            feed::decisions::retire(registers.r8, true, true);
            decision_result(
                EventFeedSource::DecisionReenabled,
                definition,
                owner(cooldown_status(registers, index))?,
                None,
                None,
                Some(days),
            );
            Ok(())
        } else if index >= 8 {
            let instance = registers.rbx;
            if index != 15 {
                feed::decisions::cut(instance);
            }
            let targeted = index >= 10;
            let status = if matches!(index, 8..=10) {
                registers.rsi
            } else {
                registers.rbp
            };
            let (definition, selector) =
                selected(&memory, BASE.load(Ordering::Acquire), instance, targeted)?;
            if index == 15 {
                if let Some((lifecycle, days)) = duration_result(&memory, instance)? {
                    let combined = feed::decisions::write_result(
                        instance,
                        Some(lifecycle),
                        Some(days),
                        EventFeedSource::DecisionDurationChanged,
                    );
                    if !combined {
                        decision_result(
                            EventFeedSource::DecisionDurationChanged,
                            definition,
                            owner(status)?,
                            selector,
                            Some(lifecycle),
                            Some(days),
                        );
                    }
                }
                return Ok(());
            }
            let (source, lifecycle, days) = terminal_operands(
                &memory,
                instance,
                index,
                registers.rcx as i32,
                registers.rax as i32,
            )?;
            let combined = feed::decisions::write_result(instance, lifecycle, days, source);
            if !combined {
                decision_result(
                    source,
                    definition,
                    owner(status)?,
                    selector,
                    lifecycle,
                    days,
                );
            }
            Ok(())
        } else {
            let instance = registers.rbx;
            feed::decisions::cut(instance);
            let base = BASE.load(Ordering::Acquire);
            if engine_sites::at(base, engine_decisions::ctimed_decision::VTABLE_RVA).is_none_or(
                |vtable| engine_native_memory::read::<u64>(&memory, instance) != Some(vtable as u64),
            ) {
                Err("mission terminal instance identity differs")
            } else {
                field::<u64>(
                    &memory,
                    instance,
                    engine_decisions::ctimed_decision::DEFINITION,
                )
                .and_then(|definition| {
                    let values = field::<[u8; 8]>(
                        &memory,
                        instance,
                        engine_decisions::ctimed_decision::DAYS,
                    )?;
                    let source = if index == 2 {
                        EventFeedSource::DecisionCompleted
                    } else {
                        EventFeedSource::DecisionTimedOut
                    };
                    let lifecycle = Some(i32::from_le_bytes(values[4..].try_into().unwrap()));
                    if !feed::decisions::write_result(instance, lifecycle, None, source) {
                        decision_result(
                            source,
                            definition as usize,
                            owner(registers.rsi)?,
                            None,
                            lifecycle,
                            None,
                        );
                    }
                    Ok(())
                })
            }
        }
    })();
    if let Err(reason) = result {
        feed::refuse(reason);
    }
}
fn cooldown_status(registers: &Registers, index: usize) -> usize {
    if index == 5 {
        registers.rbp
    } else {
        registers.r14
    }
}
fn cooldown_result(
    memory: &impl Memory,
    base: usize,
    instance: usize,
) -> Result<(usize, i32), &'static str> {
    if engine_sites::at(base, engine_decisions::cdecision_cooldown::VTABLE_RVA)
        .is_none_or(|vtable| engine_native_memory::read::<u64>(memory, instance) != Some(vtable as u64))
    {
        return Err("decision cooldown instance identity differs");
    }
    Ok((
        field::<u64>(
            memory,
            instance,
            engine_decisions::cdecision_cooldown::DEFINITION,
        )? as usize,
        field(memory, instance, engine_decisions::cdecision_cooldown::DAYS)?,
    ))
}
pub(super) fn selected(
    memory: &impl Memory,
    base: usize,
    instance: usize,
    targeted: bool,
) -> Result<(usize, Option<usize>), &'static str> {
    let (vtable, definition) = if targeted {
        (
            engine_decisions::ctargeted_decision::VTABLE_RVA,
            engine_decisions::ctargeted_decision::DEFINITION,
        )
    } else {
        (
            engine_decisions::ctimed_decision::VTABLE_RVA,
            engine_decisions::ctimed_decision::DEFINITION,
        )
    };
    if engine_sites::at(base, vtable)
        .is_none_or(|vtable| engine_native_memory::read::<u64>(memory, instance) != Some(vtable as u64))
    {
        return Err("decision terminal writer instance identity differs");
    }
    Ok((
        field::<u64>(memory, instance, definition)? as usize,
        if targeted {
            Some(
                instance
                    .checked_add(engine_decisions::ctargeted_decision::SELECTOR_VIEW)
                    .ok_or("decision terminal selector overflow")?,
            )
        } else {
            None
        },
    ))
}
fn terminal_operands(
    memory: &impl Memory,
    instance: usize,
    index: usize,
    state_operand: i32,
    duration_operand: i32,
) -> Result<(EventFeedSource, Option<i32>, Option<i32>), &'static str> {
    let source = match index {
        8 | 10 | 11 => EventFeedSource::DecisionRemoved,
        9 | 12 => EventFeedSource::DecisionCancelled,
        13 => EventFeedSource::DecisionCompleted,
        14 => EventFeedSource::DecisionTimedOut,
        _ => return Err("decision terminal writer identity differs"),
    };
    let values = match index {
        8 | 9 => (
            Some(4),
            Some(field(
                memory,
                instance,
                engine_decisions::ctimed_decision::DAYS,
            )?),
        ),
        10 | 11 => (
            Some(field(
                memory,
                instance,
                engine_decisions::ctargeted_decision::STATE,
            )?),
            Some(field(
                memory,
                instance,
                engine_decisions::ctargeted_decision::DAYS,
            )?),
        ),
        12 => (Some(2), Some(duration_operand)),
        13 | 14 => (Some(state_operand), None),
        _ => unreachable!(),
    };
    Ok((source, values.0, values.1))
}
fn duration_result(
    memory: &impl Memory,
    instance: usize,
) -> Result<Option<(i32, i32)>, &'static str> {
    let state = field(
        memory,
        instance,
        engine_decisions::ctargeted_decision::STATE,
    )?;
    if state == 0 {
        return Ok(None);
    }
    Ok(Some((
        state,
        field(memory, instance, engine_decisions::ctargeted_decision::DAYS)?,
    )))
}
/// # Safety
/// The arguments are ones the selected site's published original accepts with its ABI, and
/// every object they reach stays valid for the reads its site makes; one the original destroys
/// is an identity only once it returns.
unsafe fn remove(status: usize, definition: usize, flags: usize, index: usize) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::EventFeedProvenance);
    let mut flight = Flight::enter();
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    let before = owner(status)
        .and_then(|country| removal_counts(&memory, status, index).map(|counts| (country, counts)));
    type Original = unsafe extern "system-unwind" fn(usize, usize, usize) -> usize;
    let original: Original =
        unsafe { std::mem::transmute(ORIGINAL[index].load(Ordering::Acquire)) };
    let result = timing.original(|| unsafe { original(status, definition, flags) });
    match before.and_then(|(country, before)| {
        removal_counts(&memory, status, index).map(|after| (country, before, after))
    }) {
        Ok((country, before, after)) => match removal_edges(before, after) {
            Ok((removed, reenabled)) => {
                if matches!(index, 0 | 2) && before[0] > after[0] {
                    feed::decisions::taken_erase(status, definition, None, false);
                }
                for _ in 0..removed {
                    decision(EventFeedSource::DecisionRemoved, definition, country, None);
                }
                for _ in 0..reenabled {
                    decision(
                        EventFeedSource::DecisionReenabled,
                        definition,
                        country,
                        None,
                    );
                }
            }
            Err(reason) => feed::refuse(reason),
        },
        Err(reason) => feed::refuse(reason),
    }
    flight.returned();
    result
}
unsafe extern "system-unwind" fn remove_decision(a: usize, b: usize, c: usize) -> usize {
    unsafe { remove(a, b, c, 0) }
}
unsafe extern "system-unwind" fn remove_mission(a: usize, b: usize, c: usize) -> usize {
    unsafe { remove(a, b, c, 1) }
}
fn removal_counts(
    memory: &impl Memory,
    status: usize,
    index: usize,
) -> Result<[u32; 3], &'static str> {
    use engine_decisions::cdecision_status::*;
    let offsets = match index {
        0 => [Some(DECISIONS_TAKEN), None, None],
        1 => [Some(ACTIVE_TIMED), None, None],
        2 => [Some(DECISIONS_TAKEN), Some(REMOVE), Some(RE_ENABLE)],
        _ => return Err("decision removal writer identity differs"),
    };
    let mut counts = [0; 3];
    for (count, offset) in counts.iter_mut().zip(offsets) {
        if let Some(offset) = offset {
            *count = field(memory, status, offset + 12)?;
        }
    }
    Ok(counts)
}
fn removal_edges(before: [u32; 3], after: [u32; 3]) -> Result<(u32, u32), &'static str> {
    let mut removed = [0; 3];
    for ((result, before), after) in removed.iter_mut().zip(before).zip(after) {
        *result = before
            .checked_sub(after)
            .ok_or("decision removal container grew during its writer")?;
    }
    Ok((removed[0].max(removed[1]), removed[2]))
}
unsafe extern "system-unwind" fn clear_cooldown(status: usize, definition: usize) -> usize {
    unsafe { remove(status, definition, 0, 2) }
}
/// # Safety
/// `base` identifies the authenticated executable in this process. The caller owns these sites
/// exclusively and restores the hooks and quiesces native calls before releasing code or unloading.
pub(super) unsafe fn hooks(base: usize) -> Result<(Vec<Hook>, Vec<Tap>), String> {
    let verify = |site: native::Site| verify_hook::<16>(base, site);
    let mut hooks = Vec::new();
    let mut taps = Vec::new();
    for (index, site) in native::LIFECYCLE_TAPS.into_iter().enumerate() {
        let (at, original) = verify(site)?;
        taps.push(
            unsafe { Tap::new(at, tap, index, original) }
                .map_err(|cause| hook_failure(site, "construction", cause))?
                .named(site.label()),
        );
    }
    for (index, (site, callback)) in native::LIFECYCLE_ENTRIES
        .into_iter()
        .zip([
            remove_decision as *const () as usize,
            remove_mission as *const () as usize,
            clear_cooldown as *const () as usize,
            adjustments::display_duration as *const () as usize,
        ])
        .enumerate()
    {
        let (at, original) = verify(site)?;
        let hook = unsafe { Hook::new(at, callback, original) }
            .map_err(|cause| hook_failure(site, "construction", cause))?
            .named(site.label());
        ORIGINAL[index].store(hook.trampoline(), Ordering::Release);
        hooks.push(hook);
    }
    Ok((hooks, taps))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clearing_five_thousand_timers_demands_only_three_count_words() {
        let _globals = crate::event_feed_observer::test_globals();
        struct CountsOnly(std::cell::Cell<usize>);
        impl Memory for CountsOnly {
            fn copy(&self, at: usize, out: &mut [u8]) -> bool {
                assert_eq!(out.len(), 4);
                assert!([0x104c, 0x107c, 0x1064].contains(&at));
                self.0.set(self.0.get() + 1);
                out.copy_from_slice(&5000_u32.to_le_bytes());
                true
            }
        }
        let memory = CountsOnly(std::cell::Cell::new(0));
        assert_eq!(removal_counts(&memory, 0x1000, 2), Ok([5000; 3]));
        assert_eq!(memory.0.get(), 3);
    }
    #[test]
    fn explicit_cooldown_clear_records_cooling_taken_and_removal_only_edges() {
        let _globals = crate::event_feed_observer::test_globals();
        assert_eq!(removal_edges([0, 0, 1], [0, 0, 0]), Ok((0, 1)));
        assert_eq!(removal_edges([1, 1, 0], [0, 0, 0]), Ok((1, 0)));
        assert_eq!(removal_edges([0, 1, 0], [0, 0, 0]), Ok((1, 0)));
        assert_eq!(removal_edges([2, 3, 4], [0, 0, 0]), Ok((3, 4)));
        assert_eq!(removal_edges([1, 1, 1], [1, 1, 1]), Ok((0, 0)));
        assert!(removal_edges([0; 3], [0, 1, 0]).is_err());
        assert!(
            native::LIFECYCLE_ENTRIES
                .iter()
                .any(|site| site.name == "ClearCooldownDecision")
        );
    }
    #[test]
    fn both_ordinary_cooldown_writers_retain_the_selected_one_day_expiry() {
        let _globals = crate::event_feed_observer::test_globals();
        let mut bytes = [0_u8; 0x18];
        bytes[..8].copy_from_slice(
            &(engine_decisions::cdecision_cooldown::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        bytes[8..16].copy_from_slice(&99_u64.to_le_bytes());
        let memory = engine_native_memory::memory::SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        assert_eq!(cooldown_result(&memory, 0, 0x1000), Ok((99, 0)));
        let mut registers: Registers = unsafe { std::mem::zeroed() };
        registers.rbp = 0x2000;
        registers.r14 = 0x3000;
        assert_eq!(cooldown_status(&registers, 5), 0x2000);
        assert_eq!(cooldown_status(&registers, 7), 0x3000);
    }
    #[test]
    fn terminal_writer_uses_store_operands_and_never_reports_pre_effect_words_as_results() {
        let _globals = crate::event_feed_observer::test_globals();
        let bytes = [0xff_u8; 0x40];
        let memory = engine_native_memory::memory::SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        assert_eq!(
            terminal_operands(&memory, 0x1000, 13, 3, 99).unwrap(),
            (EventFeedSource::DecisionCompleted, Some(3), None)
        );
        assert_eq!(
            terminal_operands(&memory, 0x1000, 14, 1, 99).unwrap(),
            (EventFeedSource::DecisionTimedOut, Some(1), None)
        );
        assert_eq!(
            terminal_operands(&memory, 0x1000, 12, -1, 17).unwrap(),
            (EventFeedSource::DecisionCancelled, Some(2), Some(17))
        );
        assert_eq!(duration_result(&memory, 0x1000).unwrap(), Some((-1, -1)));
        let mut bytes = bytes;
        bytes[engine_decisions::ctargeted_decision::STATE
            ..engine_decisions::ctargeted_decision::STATE + 4]
            .copy_from_slice(&0_i32.to_le_bytes());
        let reactivated = engine_native_memory::memory::SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        assert_eq!(duration_result(&reactivated, 0x1000).unwrap(), None);
    }
    #[test]
    fn membership_and_refresh_writers_are_present_independently_of_effect_lists() {
        let _globals = crate::event_feed_observer::test_globals();
        let names = native::LIFECYCLE_TAPS.map(|site| site.name);
        assert!(names.contains(&"TakenDecisionRemoved"));
        assert!(names.contains(&"RefreshCooldownFinished"));
        assert!(names.contains(&"MissionRemoved"));
        assert!(names.contains(&"TargetedMissionRemoved"));
        assert_ne!(
            native::DECISION_COOLDOWN_FINISHED.rva,
            native::REFRESH_COOLDOWN_FINISHED.rva
        );
    }
}
