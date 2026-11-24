use super::*;
use std::collections::BTreeMap;

static ENTRY_TEST: Mutex<()> = Mutex::new(());

fn tables() -> Vec<&'static [Caller]> {
    let mut tables: Vec<&'static [Caller]> =
        native::ENTRIES.iter().map(|entry| entry.callers).collect();
    tables.extend([
        &native::DIPLOMACY_CALLERS[..],
        &native::CIVIL_WAR_CALLERS[..],
        &native::STATE_OWNER_CALLERS[..],
    ]);
    tables
}

#[test]
fn every_native_caller_is_exactly_one_replay_route() {
    let mut named = BTreeMap::new();
    for callers in tables() {
        let mut returns: Vec<_> = callers.iter().map(|caller| caller.return_rva).collect();
        returns.sort_unstable();
        returns.dedup();
        assert_eq!(returns.len(), callers.len());
        for caller in callers {
            let route = EventFeedRoute::named(caller.route)
                .unwrap_or_else(|| panic!("{} names no replay route", caller.route));
            assert!(
                named.insert(route, caller.return_rva).is_none(),
                "{route:?}"
            );
        }
    }
    for route in EventFeedRoute::ALL {
        let callerless = matches!(
            route,
            EventFeedRoute::None
                | EventFeedRoute::PeaceApplication
                | EventFeedRoute::Decision
                | EventFeedRoute::TargetedDecision
                | EventFeedRoute::DecisionTimeout
                | EventFeedRoute::ScriptedDiplomacy
                | EventFeedRoute::OnAction
        );
        assert_eq!(named.contains_key(&route), !callerless, "{route:?}");
    }
}

#[test]
fn a_caller_outside_its_list_is_counted_and_names_no_route() {
    let _guard = ENTRY_TEST.lock().unwrap();
    BASE.store(0x1_4000_0000, Ordering::Relaxed);
    let before = UNCLASSIFIED.load(Ordering::Relaxed);
    assert_eq!(
        route(&native::DIPLOMACY_CALLERS, 0x1_4000_0000 + 0x0036_c94e),
        EventFeedRoute::DeclareWarEffect
    );
    assert_eq!(UNCLASSIFIED.load(Ordering::Relaxed), before);
    assert_eq!(
        route(&native::DIPLOMACY_CALLERS, 0x1_4000_0000 + 0x0036_c949),
        EventFeedRoute::None
    );
    assert_eq!(
        route(&native::DIPLOMACY_CALLERS, 0x1_4000_0000 + 0x0036_5014),
        EventFeedRoute::None
    );
    assert_eq!(UNCLASSIFIED.load(Ordering::Relaxed), before + 2);
}

#[test]
fn nested_operations_restore_their_enclosing_context_on_return_and_unwind() {
    assert_eq!(inside(), EventFeedCause::default());
    {
        let _conference = enter(EventFeedRoute::PeaceApplication, Some(17), 0);
        let _annex = enter(EventFeedRoute::PeaceAnnex, None, 3);
        let annexed = inside();
        assert_eq!(
            (
                annexed.route,
                annexed.within,
                annexed.conference,
                annexed.flags
            ),
            (
                EventFeedRoute::PeaceAnnex,
                EventFeedRoute::PeaceApplication,
                Some(17),
                3
            )
        );
        let transfer = performed(EventFeedRoute::StateAnnex, Some(64), 1);
        assert_eq!(
            (
                transfer.route,
                transfer.within,
                transfer.conference,
                transfer.state
            ),
            (
                EventFeedRoute::StateAnnex,
                EventFeedRoute::PeaceAnnex,
                Some(17),
                Some(64)
            )
        );
        let unwound = std::panic::catch_unwind(|| {
            let _inner = enter(EventFeedRoute::AnnexEffect, None, 0);
            panic!("native unwind");
        });
        assert!(unwound.is_err());
        assert_eq!(inside(), annexed);
    }
    assert_eq!(inside(), EventFeedCause::default());
}

static SEEN: Mutex<Option<EventFeedCause>> = Mutex::new(None);

unsafe extern "system-unwind" fn witness(
    a: usize,
    _: usize,
    _: usize,
    _: usize,
    _: usize,
    _: usize,
    _: usize,
    h: usize,
) -> usize {
    *SEEN.lock().unwrap() = Some(inside());
    a ^ h
}

#[test]
fn a_join_runs_its_original_inside_its_route_and_operands() {
    let _globals = crate::event_feed_observer::test_globals();
    let _guard = ENTRY_TEST.lock().unwrap();
    BASE.store(0x1_4000_0000, Ordering::Relaxed);
    ORIGINAL[JOIN_WAR].store(witness as *const () as usize, Ordering::Release);
    let _decision = politics(PoliticsScope::Decision, 0).unwrap();
    let _outer = politics(PoliticsScope::Diplomacy, 0x1_4000_0000 + 0x0118_acbc).unwrap();
    let unread = UNREAD.load(Ordering::Relaxed);
    let result = unsafe {
        body(
            0x55,
            0,
            0,
            3,
            0x1_0000_0001,
            0,
            0,
            0x0f,
            JOIN_WAR,
            0x1_4000_0000 + 0x0118_e40f,
        )
    };
    assert_eq!(result, 0x5a);
    assert_eq!(UNREAD.load(Ordering::Relaxed), unread + 1);
    assert_eq!(
        SEEN.lock().unwrap().take(),
        Some(EventFeedCause {
            route: EventFeedRoute::JoinAlly,
            within: EventFeedRoute::DiplomaticCommand,
            conference: None,
            state: None,
            ally: None,
            reason: Some(3),
            flags: 1,
        })
    );
    assert_eq!(inside().route, EventFeedRoute::DiplomaticCommand);
    assert_eq!(inside().within, EventFeedRoute::Decision);
    assert_eq!(active(), 0);
}

#[test]
fn entry_shims_unwind_to_the_native_return_with_the_original_stack() {
    use std::ptr;
    use windows_sys::Win32::System::Diagnostics::Debug::{
        CONTEXT, RtlLookupFunctionEntry, RtlVirtualUnwind,
    };
    for entry in CALLBACKS {
        let entry = entry as *const () as usize;
        let first = unsafe { std::slice::from_raw_parts(entry as *const u8, 7) };
        assert_eq!(first, &[0x49, 0x89, 0xe2, 0x48, 0x83, 0xec, 0x58]);
        let pc = entry as u64 + 7;
        let mut image_base = 0;
        let function = unsafe { RtlLookupFunctionEntry(pc, &raw mut image_base, ptr::null_mut()) };
        assert!(!function.is_null());
        assert_eq!(
            image_base + unsafe { (*function).BeginAddress } as u64,
            entry as u64
        );
        #[repr(align(16))]
        struct Stack([u64; 16]);
        let mut stack = Stack([0; 16]);
        let return_pc = 0x1_4000_0000 + native::JOIN_WAR.callers[0].return_rva.0 as u64;
        stack.0[11] = return_pc;
        let mut context: CONTEXT = unsafe { std::mem::zeroed() };
        context.Rip = pc;
        context.Rsp = stack.0.as_ptr() as u64;
        context.Rbx = 77;
        let mut handler_data = ptr::null_mut();
        let mut establisher = 0;
        unsafe {
            RtlVirtualUnwind(
                0,
                image_base,
                pc,
                function,
                &raw mut context,
                &raw mut handler_data,
                &raw mut establisher,
                ptr::null_mut(),
            );
        }
        assert_eq!(context.Rip, return_pc);
        assert_eq!(context.Rsp, stack.0.as_ptr() as u64 + 0x60);
        assert_eq!(context.Rbx, 77);
    }
}
