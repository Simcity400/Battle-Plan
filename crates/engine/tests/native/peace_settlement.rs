use engine_native_memory::memory::SliceMemory;
use engine_peace::{
    peace_action, peace_conference as c, peace_settlement as p, peace_settlement::Action,
    peace_settlement::Subject, peace_settlement::Transfer,
};

use crate::support::peace_world::Fixture;

use engine_native_memory::walk::Buffer;

#[test]
fn peace_owned_native_order_mod_tokens_and_statuses() {
    let mut f = Fixture::new(2, 5);
    f.u32(0x20000 + 0x70 + peace_action::STATUS, 3);
    f.u32(0x20000 + 3 * 0x70 + peace_action::STATUS, 4);
    let mut actions = vec![Action::default(); 5];
    let mut winners = vec![0; 2];
    let snapshot = f.copy(Buffer::Growing(&mut winners), Buffer::Growing(&mut actions));
    assert_eq!(snapshot.failure, None);
    assert_eq!(snapshot.actions, 3);
    assert_eq!(snapshot.omitted, 0);
    assert_eq!(winners, [0xa0000000, 0xa0000001]);
    assert_eq!(
        actions[..3].iter().map(|a| a.ordinal).collect::<Vec<_>>(),
        [0, 2, 4]
    );
    assert_eq!(actions[2].operands.token, 0x90000004);
    assert_eq!(actions[0].operands.flags, Some([1, 2, 3, 4]));
    assert_eq!(actions[0].operands.negotiator, 0xc0000000);
}

#[test]
fn peace_authentication_short_reads_and_cycles_never_become_empty_success() {
    let mut f = Fixture::new(0, 1);
    f.u64(0x20000, 99);
    assert_eq!(
        f.copy(Buffer::Count, Buffer::Growing(&mut vec![Action::default()])).failure,
        Some("peace action vtable unsupported")
    );
    f.u64(0x20000, engine_peace::ctake_state_action::VTABLE_RVA.0 as u64);
    f.array(0x910, 0xa00, 1, 8);
    f.u64(0xa00, 0x20000);
    f.bytes.truncate(0x20000 + 0x4f);
    assert_eq!(
        f.copy(Buffer::Count, Buffer::Growing(&mut vec![Action::default()])).failure,
        Some("peace action operands unreadable")
    );
    let mut f = Fixture::new(0, 1);
    f.u64(0x610, 0x600);
    assert!(f.copy(Buffer::Count, Buffer::Growing(&mut vec![Action::default()])).failure.is_some());
}

#[test]
fn peace_concrete_operands_respect_the_government_allocation() {
    let mut f = Fixture::new(0, 1);
    for (vtable, form) in [
        (
            engine_peace::cliberate_country_action::VTABLE_RVA.0,
            peace_action::Form::Liberate,
        ),
        (
            engine_peace::cpuppet_country_action::VTABLE_RVA.0,
            peace_action::Form::Puppet,
        ),
        (
            engine_peace::cforce_government_action::VTABLE_RVA.0,
            peace_action::Form::Government,
        ),
    ] {
        f.u64(0x20000, vtable as u64);
        f.u32(0x20050, 0xf0000000);
        f.u64(0x20058, u64::MAX);
        let length = if form == peace_action::Form::Government {
            0x20058
        } else {
            f.bytes.len()
        };
        let row = peace_action::copy(
            &SliceMemory {
                base: 0,
                bytes: &f.bytes[..length],
            },
            0,
            0x20000,
        )
        .unwrap();
        assert_eq!(row.form, form);
        assert_eq!(row.country_operand, Some(0xf0000000));
        assert_eq!(
            row.puppet_auxiliary,
            if form == peace_action::Form::Puppet {
                Some(u64::MAX)
            } else {
                None
            }
        );
    }
}

#[test]
fn peace_countries_and_actions_scale_linearly_at_design_and_stress_sizes() {
    for size in [1500, 5000] {
        let f = Fixture::new(size, size);
        let mut winners = vec![0; size];
        let mut actions = vec![Action::default(); f.actions];
        let result = f.copy(Buffer::Growing(&mut winners), Buffer::Growing(&mut actions));
        assert_eq!(result.failure, None);
        assert_eq!(result.winners, size);
        assert_eq!(result.actions, size);
        assert_eq!(actions[size - 1].ordinal, (size - 1) as u32);
    }
}

#[test]
fn peace_excluded_bids_never_fill_or_truncate_a_settlement() {
    let mut f = Fixture::new(0, 3);
    for (i, status) in [(1, 3), (2, 4)] {
        f.u32(0x20000 + i * 0x70 + peace_action::STATUS, status);
    }
    f.u64(
        0x20000 + 2 * 0x70,
        engine_peace::ctake_navy_peace_action::VTABLE_RVA.0 as u64,
    );
    f.u64(
        0x20000 + 2 * 0x70 + engine_peace::ctake_navy_peace_action::SHIP,
        0xdead_0000,
    );
    let mut actions = vec![Action::default()];
    let result = f.copy(Buffer::Count, Buffer::Growing(&mut actions));
    assert_eq!(result.failure, None);
    assert_eq!((result.actions, result.omitted), (1, 0));
    assert_eq!(actions[0].ordinal, 0);
    f.u32(0x20000 + peace_action::STATUS, 3);
    let result = f.copy(Buffer::Count, Buffer::Count);
    assert_eq!(result.failure, None);
    assert_eq!((result.actions, result.omitted), (0, 0));
    f.u64(0x20000 + 0x70, 99);
    assert_eq!(
        f.copy(Buffer::Count, Buffer::Count).failure,
        Some("peace action vtable unsupported")
    );
    f.u64(
        0x20000 + 0x70,
        engine_peace::ctake_state_action::VTABLE_RVA.0 as u64,
    );
    f.bytes[0x100 + c::FINALIZED] = 0;
    let bids = p::copy_bids(
        &SliceMemory {
            base: 0,
            bytes: &f.bytes,
        },
        0,
        0x100,
        p::Output {
            winners: engine_native_memory::walk::Buffer::Count,
            losers: engine_native_memory::walk::Buffer::Count,
            actions: engine_native_memory::walk::Buffer::Count,
            transfers: engine_native_memory::walk::Buffer::Count,
            subjects: engine_native_memory::walk::Buffer::Count,
            name_bytes: engine_native_memory::walk::Buffer::Count,
        },
    );
    assert_eq!(bids.failure, None);
    assert_eq!((bids.actions, bids.omitted), (0, 3));
}

#[test]
fn peace_tree_cycle_budget_follows_the_live_header_size() {
    use std::cell::Cell;
    struct Count<'a> {
        memory: SliceMemory<'a>,
        calls: Cell<usize>,
    }
    impl engine_native_memory::Memory for Count<'_> {
        fn copy(&self, at: usize, bytes: &mut [u8]) -> bool {
            self.calls.set(self.calls.get() + 1);
            self.memory.copy(at, bytes)
        }
    }
    let mut f = Fixture::new(0, 0);
    f.u64(0x600, 0x600);
    f.u64(0x610, 0x600);
    let memory = Count {
        memory: SliceMemory {
            base: 0,
            bytes: &f.bytes,
        },
        calls: Cell::new(0),
    };
    let result = p::copy(
        &memory,
        0,
        0x100,
        p::Output {
            winners: engine_native_memory::walk::Buffer::Count,
            losers: engine_native_memory::walk::Buffer::Count,
            actions: engine_native_memory::walk::Buffer::Count,
            transfers: engine_native_memory::walk::Buffer::Count,
            subjects: engine_native_memory::walk::Buffer::Count,
            name_bytes: engine_native_memory::walk::Buffer::Count,
        },
    );
    assert_eq!(result.failure, Some("peace history cycle"));
    assert!(
        memory.calls.get() < 30,
        "small corrupt trees must stop quickly"
    );
}

#[test]
fn peace_action_operands_use_one_scattered_span_after_authentication() {
    use std::cell::Cell;
    struct Span<'a> {
        memory: SliceMemory<'a>,
        spans: Cell<usize>,
        scalars: Cell<usize>,
    }
    impl engine_native_memory::Memory for Span<'_> {
        fn copy(&self, at: usize, bytes: &mut [u8]) -> bool {
            self.scalars.set(self.scalars.get() + 1);
            self.memory.copy(at, bytes)
        }
        fn copy_scattered<const N: usize>(
            &self,
            at: [usize; N],
            bytes: &mut [&mut [u8]; N],
        ) -> usize {
            assert_eq!(N, 1);
            self.spans.set(self.spans.get() + 1);
            self.memory.copy_scattered(at, bytes)
        }
    }
    let mut f = Fixture::new(0, 1);
    f.u64(
        0x20000,
        engine_peace::cpuppet_country_action::VTABLE_RVA.0 as u64,
    );
    let memory = Span {
        memory: SliceMemory {
            base: 0,
            bytes: &f.bytes,
        },
        spans: Cell::new(0),
        scalars: Cell::new(0),
    };
    assert_eq!(
        peace_action::copy(&memory, 0, 0x20000).unwrap().form,
        peace_action::Form::Puppet
    );
    assert_eq!(memory.spans.get(), 1);
    assert_eq!(memory.scalars.get(), 1);
}

#[test]
fn peace_guarded_fixture_copies_populated_transfer_arrays() {
    let mut f = Fixture::new(2, 3);
    f.populate_transfers(3);
    let (base, pointer) = f.rebase_image(91);
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    let mut transfers = vec![Transfer::default(); 3];
    let result = p::copy(
        &memory,
        base,
        pointer,
        p::Output {
            winners: engine_native_memory::walk::Buffer::Growing(&mut vec![0; 2]),
            losers: engine_native_memory::walk::Buffer::Count,
            actions: engine_native_memory::walk::Buffer::Growing(&mut vec![Action::default(); 3]),
            transfers: engine_native_memory::walk::Buffer::Growing(&mut transfers),
            subjects: engine_native_memory::walk::Buffer::Count,
            name_bytes: engine_native_memory::walk::Buffer::Count,
        },
    );
    assert_eq!(result.failure, None);
    assert_eq!(result.omitted, 0);
    assert_eq!(result.transfers, 3);
    assert_eq!(transfers.iter().map(|t| t.state).collect::<Vec<_>>(), [0, 1, 2]);
}

#[test]
fn peace_resident_bids_keep_rejected_actions_and_navy_references_are_owned() {
    let mut f = Fixture::new(0, 2);
    f.bytes[0x100 + c::FINALIZED] = 0;
    f.u32(0x20000 + peace_action::STATUS, 3);
    let mut actions = vec![Action::default(); 2];
    let row = p::copy_bids(
        &SliceMemory {
            base: 0,
            bytes: &f.bytes,
        },
        0,
        0x100,
        p::Output {
            winners: engine_native_memory::walk::Buffer::Count,
            losers: engine_native_memory::walk::Buffer::Count,
            actions: engine_native_memory::walk::Buffer::Growing(&mut actions),
            transfers: engine_native_memory::walk::Buffer::Count,
            subjects: engine_native_memory::walk::Buffer::Count,
            name_bytes: engine_native_memory::walk::Buffer::Count,
        },
    );
    assert_eq!(row.failure, None);
    assert_eq!(row.actions, 2);
    assert_eq!(actions[0].operands.status, 3);
    assert_eq!(
        f.copy(Buffer::Count, Buffer::Growing(&mut actions)).failure,
        Some("peace settlement finalized byte differs")
    );

    let literal = engine_peace::ctake_navy_peace_action::LABEL_LITERAL_RVA.0;
    f.bytes.resize(literal + 32, 0);
    f.bytes[literal..literal + 12].copy_from_slice(b"modded_navy\0");
    f.u64(
        0x20000,
        engine_peace::ctake_navy_peace_action::VTABLE_RVA.0 as u64,
    );
    f.u64(0x20048, 0xe00);
    f.u64(0xe00, engine_native::cship::VTABLE_RVA.0 as u64);
    f.u64(0xe08, 0x9900000011);
    f.u64(0x20050, (-3_i64) as u64);
    let navy = peace_action::copy(
        &SliceMemory {
            base: 0,
            bytes: &f.bytes,
        },
        0,
        0x20000,
    )
    .unwrap();
    assert_eq!(navy.ship, Some(0x9900000011));
    assert_eq!(navy.ratio, Some(-3));
    assert_eq!(navy.state, None);
    f.u64(0xe00, 77);
    assert_eq!(
        peace_action::copy(
            &SliceMemory {
                base: 0,
                bytes: &f.bytes
            },
            0,
            0x20000
        ),
        Err("peace selected ship vtable mismatch")
    );
}

#[test]
fn peace_automatic_transfers_are_separate_and_owned() {
    let mut f = Fixture::new(0, 0);
    f.array(0x100 + c::WINNER_PARTICIPANTS, 0xa00, 1, 8);
    f.u64(0xa00, 0xb00);
    f.u64(
        0xb00,
        engine_peace::conference_winner_participant::VTABLE_RVA.0 as u64,
    );
    f.u32(0xb08, 0xe0000001);
    f.array(
        0xb00 + engine_peace::conference_winner_participant::AUTOMATIC_STATES,
        0xd00,
        2,
        4,
    );
    f.u32(0xd00, 7777);
    f.u32(0xd04, 42);
    let mut transfers = vec![Transfer::default(); 2];
    let result = p::copy(
        &SliceMemory {
            base: 0,
            bytes: &f.bytes,
        },
        0,
        0x100,
        p::Output {
            winners: engine_native_memory::walk::Buffer::Count,
            losers: engine_native_memory::walk::Buffer::Count,
            actions: engine_native_memory::walk::Buffer::Count,
            transfers: engine_native_memory::walk::Buffer::Growing(&mut transfers),
            subjects: engine_native_memory::walk::Buffer::Growing(&mut vec![Subject::default()]),
            name_bytes: engine_native_memory::walk::Buffer::Count,
        },
    );
    assert_eq!(result.failure, None);
    assert_eq!(result.actions, 0);
    assert_eq!(
        transfers.iter().map(|t| (t.state, t.taker, t.automatic)).collect::<Vec<_>>(),
        [(7777, 0xe0000001, true), (42, 0xe0000001, true)]
    );
}

#[test]
fn peace_native_headers_and_full_capture_exceed_former_limits() {
    let countries = 65_537;
    let count = 262_145;
    let mut f = Fixture::new(countries, count);
    f.populate_transfers(count);
    let memory = f.memory();
    let demand = p::demand(&memory, 0, 0x100);
    assert_eq!(demand.failure, None);
    assert_eq!(
        (demand.winners, demand.actions, demand.transfers),
        (countries, count, count)
    );
    let mut actions = vec![Action::default(); demand.actions];
    let mut transfers = vec![Transfer::default(); demand.transfers];
    let mut winners = vec![0; demand.winners];
    let copied = p::copy(
        &memory,
        0,
        0x100,
        p::Output {
            winners: engine_native_memory::walk::Buffer::Growing(&mut winners),
            losers: engine_native_memory::walk::Buffer::Count,
            actions: engine_native_memory::walk::Buffer::Growing(&mut actions),
            transfers: engine_native_memory::walk::Buffer::Growing(&mut transfers),
            subjects: engine_native_memory::walk::Buffer::Count,
            name_bytes: engine_native_memory::walk::Buffer::Count,
        },
    );
    assert_eq!(copied, demand);
    assert_eq!(actions.last().unwrap().ordinal, (count - 1) as u32);
    assert_eq!(transfers.last().unwrap().state, (count - 1) as i32);
    assert_eq!(winners.last(), Some(&(0xa0000000 + (countries - 1) as u32)));
}

#[test]
fn peace_rejected_actions_do_not_limit_native_scan_work() {
    let mut f = Fixture::new(0, 262_145);
    for i in 0..f.actions - 1 {
        f.u32(0x20000 + i * 0x70 + peace_action::STATUS, 3);
    }
    let mut actions = vec![Action::default()];
    let copied = f.copy(Buffer::Count, Buffer::Growing(&mut actions));
    assert_eq!(copied.failure, None);
    assert_eq!(copied.omitted, 0);
    assert_eq!(copied.actions, 1);
    assert_eq!(actions[0].ordinal, 262_144);
}

#[test]
fn peace_empty_history_and_turns_follow_native_sizes() {
    let count = 65_537;
    let mut f = Fixture::new(0, 0);
    let nodes = f.bytes.len();
    f.bytes.resize(nodes + count * 0x38, 0);
    f.u64(0x108 + c::HISTORY, count as u64);
    f.u64(0x500, nodes as u64);
    for i in 0..count {
        let node = nodes + i * 0x38;
        f.u64(node, 0x500);
        f.u64(node + 8, if i == 0 { 0x500 } else { (node - 0x38) as u64 });
        f.u64(
            node + 16,
            if i + 1 == count {
                0x500
            } else {
                (node + 0x38) as u64
            },
        );
        f.u32(node + engine_peace::peace_history_node::COUNTRY, i as u32);
    }
    let copied = f.copy(Buffer::Count, Buffer::Count);
    assert_eq!(copied.failure, None);
    assert_eq!(copied.omitted, 0);
    let mut f = Fixture::new(0, 0);
    let turns = f.bytes.len();
    let count = 262_145;
    f.bytes.resize(turns + count * 8, 0);
    f.array(0x628, turns, count, 8);
    for i in 0..count {
        f.u64(turns + i * 8, 0x900);
    }
    let copied = f.copy(Buffer::Count, Buffer::Count);
    assert_eq!(copied.failure, None);
    assert_eq!(copied.omitted, 0);
}

#[test]
fn peace_beneficiaries_and_automatic_transfers_have_no_chosen_limit() {
    let mut f = Fixture::new(0, 0);
    let participants = f.bytes.len();
    let count = 65_537;
    f.bytes.resize(participants + count * 8, 0);
    f.array(0x100 + c::LIBERATED_PARTICIPANTS, participants, count, 8);
    f.u64(
        0xb00,
        engine_peace::conference_liberated_participant::VTABLE_RVA.0 as u64,
    );
    for i in 0..count {
        f.u64(participants + i * 8, 0xb00);
    }
    let copied = f.copy(Buffer::Count, Buffer::Count);
    assert_eq!(copied.failure, None);
    assert_eq!(copied.omitted, 0);
    f.array(0x100 + c::LIBERATED_PARTICIPANTS, 0, 0, 8);
    f.array(0x100 + c::WINNER_PARTICIPANTS, 0xa00, 1, 8);
    f.u64(0xa00, 0xb00);
    f.u64(
        0xb00,
        engine_peace::conference_winner_participant::VTABLE_RVA.0 as u64,
    );
    let states = f.bytes.len();
    let count = 262_145;
    f.bytes.resize(states + count * 4, 0);
    f.array(
        0xb00 + engine_peace::conference_winner_participant::AUTOMATIC_STATES,
        states,
        count,
        4,
    );
    let mut transfers = vec![Transfer::default(); count];
    let copied = p::copy(
        &SliceMemory {
            base: 0,
            bytes: &f.bytes,
        },
        0,
        0x100,
        p::Output {
            winners: engine_native_memory::walk::Buffer::Count,
            losers: engine_native_memory::walk::Buffer::Count,
            actions: engine_native_memory::walk::Buffer::Count,
            transfers: engine_native_memory::walk::Buffer::Growing(&mut transfers),
            subjects: engine_native_memory::walk::Buffer::Count,
            name_bytes: engine_native_memory::walk::Buffer::Count,
        },
    );
    assert_eq!(copied.failure, None);
    assert_eq!(copied.omitted, 0);
    assert_eq!(copied.transfers, count);
    assert!(transfers.iter().all(|transfer| transfer.automatic));
}

#[test]
fn peace_navy_name_reads_only_the_evidenced_literal_storage() {
    let mut f = Fixture::new(0, 1);
    let literal = engine_peace::ctake_navy_peace_action::LABEL_LITERAL_RVA.0;
    f.bytes.resize(
        literal + engine_peace::ctake_navy_peace_action::LABEL_LITERAL_BYTES,
        0,
    );
    f.bytes[literal..].copy_from_slice(b"synthetic_navy_name_2\0");
    f.u64(
        0x20000,
        engine_peace::ctake_navy_peace_action::VTABLE_RVA.0 as u64,
    );
    f.u64(0x20048, 0);
    let row = peace_action::copy(
        &SliceMemory {
            base: 0,
            bytes: &f.bytes,
        },
        0,
        0x20000,
    )
    .unwrap();
    assert_eq!(
        row.name_key.as_ref().unwrap().as_slice(),
        b"synthetic_navy_name_2\0"
    );
}

#[test]
fn peace_demand_refuses_unbacked_array_counts() {
    for offset in [
        0x100 + c::WINNERS,
        0x100 + c::LOSERS,
        0x10000 + engine_native::cgame_state::COUNTRIES,
        0x628,
        0x910,
        0x100 + c::WINNER_PARTICIPANTS,
        0xb00 + engine_peace::conference_beneficiary_participant::STATES,
        0xb00 + engine_peace::conference_winner_participant::AUTOMATIC_STATES,
    ] {
        let mut f = Fixture::new(1, 1);
        f.populate_transfers(1);
        f.array(offset, 0x20000, i32::MAX as usize, 8);
        let demand = p::demand(&f.memory(), 0, 0x100);
        assert_eq!(
            demand.failure,
            Some("peace settlement array element unreadable"),
            "unmapped tail at header {offset:#x} must not become demand",
        );
    }
}

#[test]
fn peace_demand_refuses_history_larger_than_country_registry() {
    let mut f = Fixture::new(1, 0);
    f.u64(0x108 + c::HISTORY, 2);
    assert_eq!(
        p::demand(&f.memory(), 0, 0x100).failure,
        Some("peace history exceeds country registry"),
    );
}

#[test]
fn peace_demand_stops_revisited_history_links_before_the_size_budget() {
    use std::cell::Cell;
    struct Count<M> {
        memory: M,
        calls: Cell<usize>,
    }
    impl<M: engine_native_memory::Memory> engine_native_memory::Memory for Count<M> {
        fn copy(&self, at: usize, output: &mut [u8]) -> bool {
            self.calls.set(self.calls.get() + 1);
            self.memory.copy(at, output)
        }
    }
    let mut f = Fixture::new(65_537, 0);
    f.u64(0x108 + c::HISTORY, 65_537);
    f.u64(0x600, 0x600);
    f.u64(0x610, 0x600);
    let memory = Count {
        memory: f.memory(),
        calls: Cell::new(0),
    };
    assert_eq!(
        p::demand(&memory, 0, 0x100).failure,
        Some("peace history cycle")
    );
    assert!(
        memory.calls.get() < 20,
        "a revisit must stop without walking the claimed size"
    );
}

#[test]
fn peace_demand_accepts_large_valid_history_and_turn_arrays() {
    let count = 65_537;
    let mut f = Fixture::new(count, 0);
    let nodes = f.bytes.len();
    f.bytes.resize(nodes + count * 0x38, 0);
    f.u64(0x108 + c::HISTORY, count as u64);
    f.u64(0x500, nodes as u64);
    for i in 0..count {
        let node = nodes + i * 0x38;
        f.u64(node, 0x500);
        f.u64(node + 8, if i == 0 { 0x500 } else { (node - 0x38) as u64 });
        f.u64(
            node + 16,
            if i + 1 == count {
                0x500
            } else {
                (node + 0x38) as u64
            },
        );
        f.u32(node + engine_peace::peace_history_node::COUNTRY, i as u32);
    }
    let demand = p::demand(&f.memory(), 0, 0x100);
    assert_eq!(demand.failure, None);
    assert_eq!(demand.winners, count);
    let mut f = Fixture::new(1, 0);
    let turns = f.bytes.len();
    let count = 262_145;
    f.bytes.resize(turns + count * 8, 0);
    f.array(0x628, turns, count, 8);
    for i in 0..count {
        f.u64(turns + i * 8, 0x900);
    }
    let demand = p::demand(&f.memory(), 0, 0x100);
    assert_eq!(demand.failure, None);
    assert_eq!(demand.actions, 0);
}
