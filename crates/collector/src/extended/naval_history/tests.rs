use super::*;

fn owned_failure_fixture() -> Captured {
    Captured::with_capacities(&Capacities {
        rows: [1; N],
        theatre_names: 8,
        sunk_names: 8,
        orders_sources: 1,
        wing_sources: 1,
        roster_scratch: 8,
    })
}

/// The OOB theatre memo keeps first-seen root order, whatever the
/// addresses' order, exactly as the former sorted-insert memo did.
#[test]
fn a_large_registry_presizes_the_order_of_battle_theatres() {
    let _hint = crate::country_scale::hint_guard();
    crate::country_scale::note_slots(5_000);
    let mut job = NavalJob::new();
    job.presize_countries();
    let want = crate::country_scale::rows(5_000, 1) * OOB_THEATRE_REFERENCES_PER_COUNTRY;
    assert!(job.oob_theatres.capacity() >= want);
    assert!(job.oob_theatre_positions.capacity() >= want);
}

/// The registry census snapshot holds only a complete census that fits,
/// grows off the callback from its demand, and is dropped with the pass.
#[test]
fn census_roots_snapshot_fits_grows_and_never_outlives_its_pass() {
    let roots = [corders_group::Root::default(); 5];
    let mut job = NavalJob::default();
    job.capture_roots(&roots, Outcome::Rows { len: 3 });
    assert_eq!((job.roots_len, job.roots.capacity()), (None, 0));
    assert!(job.grow_for_next());
    job.grow();
    job.capture_roots(&roots, Outcome::Rows { len: 3 });
    assert_eq!(job.roots_len, Some(3));
    assert_eq!(job.roots, roots[..3]);
    job.capture_roots(
        &roots,
        Outcome::Truncated {
            len: 3,
            unread_rows: 4,
        },
    );
    assert_eq!(job.roots_len, None);
    assert_eq!(job.roots_required, 7);
    job.capture_roots(&roots, Outcome::Unavailable { reason: "torn" });
    assert_eq!(job.roots_len, None);
    job.capture_roots(&roots, Outcome::Rows { len: 3 });
    job.clear();
    assert_eq!((job.roots_len, job.roots.len()), (None, 0));
    // A census longer than the roots it names is not kept.
    job.capture_roots(&roots[..2], Outcome::Rows { len: 3 });
    assert_eq!(job.roots_len, None);
}

#[test]
fn oob_theatre_dedup_keeps_first_occurrences_in_root_order() {
    let first_seen = |values: &[usize]| {
        let mut seen = Vec::new();
        for &value in values {
            if !seen.contains(&value) {
                seen.push(value);
            }
        }
        seen
    };
    for values in [
        vec![],
        vec![7],
        vec![7, 7, 7],
        vec![0x30, 0x10, 0x30, 0x20, 0x10, 0x40, 0x20],
        vec![5, 4, 3, 2, 1, 1, 2, 3, 4, 5],
        (0..500).map(|i| (i * 7919) % 97 * 0x40).collect(),
    ] {
        let mut deduped = values.clone();
        let mut positions = Vec::with_capacity(values.len());
        let capacity = positions.capacity();
        dedup_first_seen(&mut deduped, &mut positions);
        assert_eq!(deduped, first_seen(&values));
        assert_eq!(positions.capacity(), capacity);
    }
}

#[test]
fn owned_supremacy_and_sunk_failures_are_table_local() {
    let mut captured = owned_failure_fixture();
    captured.raw_mode = true;
    captured.supremacy_raw_mode = true;
    captured.sunk_raw_mode = true;
    captured.supremacy.outcome = Outcome::Rows { len: 1 };
    captured.sunk.outcome = Outcome::Rows { len: 1 };
    captured.convoys.outcome = Outcome::Rows { len: 1 };
    captured.sunk_names.len = 4;
    captured.sunk_names.required = 9;
    let bytes = captured.allocated_bytes();
    captured.fail_supremacy_capture("supremacy worker failed");
    assert_eq!(
        captured.supremacy.outcome,
        Outcome::Unavailable {
            reason: "supremacy worker failed"
        }
    );
    assert!(!captured.supremacy_raw_mode);
    assert!(captured.sunk_raw_mode && captured.raw_mode);
    assert_eq!(captured.sunk.outcome, Outcome::Rows { len: 1 });
    captured.fail_sunk_capture("sunk worker failed");
    assert_eq!(
        captured.sunk.outcome,
        Outcome::Unavailable {
            reason: "sunk worker failed"
        }
    );
    assert_eq!(
        (captured.sunk_names.len, captured.sunk_names.required),
        (0, 0)
    );
    assert!(!captured.sunk_raw_mode && captured.raw_mode);
    assert_eq!(captured.convoys.outcome, Outcome::Rows { len: 1 });
    assert_eq!(captured.allocated_bytes(), bytes);
}

#[test]
fn owned_naval_failure_preserves_independent_sources_and_other_families() {
    let mut captured = owned_failure_fixture();
    captured.raw_mode = true;
    captured.supremacy_raw_mode = true;
    captured.sunk_raw_mode = true;
    captured.supremacy.outcome = Outcome::Rows { len: 1 };
    captured.convoys.outcome = Outcome::Rows { len: 1 };
    captured.cooldowns.outcome = Outcome::Rows { len: 1 };
    captured.edges.outcome = Outcome::Rows { len: 1 };
    captured.results.outcome = Outcome::Rows { len: 1 };
    captured.result_members.outcome = Outcome::Rows { len: 1 };
    captured.result_countries.outcome = Outcome::Rows { len: 1 };
    captured.sunk.outcome = Outcome::Rows { len: 1 };
    captured.orders_sources.outcome = Outcome::Rows { len: 1 };
    captured.wing_sources.outcome = Outcome::Rows { len: 1 };
    captured.orders_finished = true;
    captured.wings_finished = true;
    captured.source_ticks = [17, 23, 29];
    captured.theatre_names.len = 4;
    captured.theatre_names.required = 9;
    let mut outcomes = captured.outcomes();
    let bytes = captured.allocated_bytes();
    captured.fail_naval_capture("naval worker failed");
    for index in [4, 5, 6, 7, 8, 9, 14, 15, 16] {
        outcomes[index] = Outcome::Unavailable {
            reason: "naval worker failed",
        };
    }
    assert_eq!(captured.outcomes(), outcomes);
    assert_eq!(captured.orders_sources.outcome, Outcome::Rows { len: 1 });
    assert_eq!(captured.wing_sources.outcome, Outcome::Rows { len: 1 });
    assert!(captured.orders_finished);
    assert!(captured.wings_finished);
    assert_eq!(captured.source_ticks, [17, 23, 29]);
    assert_eq!(captured.theatre_names.len, 0);
    assert_eq!(captured.theatre_names.required, 0);
    assert!(!captured.raw_mode);
    assert!(captured.supremacy_raw_mode && captured.sunk_raw_mode);
    assert_eq!(captured.allocated_bytes(), bytes);
}

#[test]
fn naval_job_capacity_accounting_tracks_all_vectors_through_growth_clear_and_drop() {
    use crate::raw_memory::{Capacity, Charge, Tracker, vector_bytes};
    static LOCAL: Tracker = Tracker::new();
    let mut job = NavalJob::default();
    job.wing_sources.reserve_exact(3);
    job.capacity_charge = Charge::empty_on(&LOCAL);
    job.update_capacity_charge();
    let expected = |job: &NavalJob| vector_bytes(&job.wing_sources);
    let initial = LOCAL.snapshot().current.auxiliary_vectors;
    assert_eq!(initial, expected(&job));
    job.wing_outcome = Some(Outcome::Truncated {
        len: 3,
        unread_rows: 5,
    });
    job.grow();
    let grown = LOCAL.snapshot().current.auxiliary_vectors;
    assert!(grown > initial);
    assert_eq!(grown, expected(&job));
    job.clear();
    assert_eq!(LOCAL.snapshot().current.auxiliary_vectors, grown);
    drop(job);
    assert_eq!(LOCAL.snapshot().current, Capacity::default());
}

#[test]
fn shared_naval_projection_preserves_truncated_prefix_and_growth() {
    use crate::extended::air::{AirWingCopyContext, AirWingJob, wing_test_kit as kit};
    let mut bytes = kit::fixture();
    bytes[0x34000..0x34008].fill(0);
    let memory = kit::memory(&bytes);
    let mut naval = NavalJob::default();
    naval.wing_sources.reserve_exact(1);
    let mut wings = AirWingJob::new();
    let context = AirWingCopyContext {
        hour: 0,
        force_full: true,
        base: kit::BASE,
        game_state: kit::GAME_STATE,

        limits: [2, 3, 2],
        naval_limit: naval.wing_limit(),
    };
    assert!(crate::extended::air::copy_air_refresh(&memory, context, &mut wings).raw_ok);
    naval.install_wings(&wings);
    assert_eq!(naval.wing_sources.len(), 1);
    assert_eq!(
        naval.wing_outcome,
        Some(Outcome::Truncated {
            len: 1,
            unread_rows: 1
        })
    );
    assert_eq!(naval.wing_required, 2);
    assert!(naval.grow_for_next());
    naval.grow();
    assert!(naval.wing_limit() >= 2);
}

#[test]
fn naval_decode_bounds_shared_memberships_to_frame_capacity() {
    use super::naval_test_kit as kit;
    let bytes = kit::fixture();
    let memory = kit::memory(&bytes);
    let mut job = NavalJob::new();
    kit::stage_wings(&memory, &mut job);
    let context = NavalCopyContext {
        hour: 0,
        force_full: true,
        base: kit::BASE,
        game_state: kit::GAME_STATE,
    };
    assert!(super::copy_naval_refresh(&memory, context, &mut job).raw_ok);
    job.wing_sources.resize(2, Default::default());
    let capacity = Capacities {
        wing_sources: 1,
        ..Default::default()
    };
    let mut captured = Captured::with_capacities(&capacity);
    for missing in [0, 3] {
        job.wing_outcome = Some(if missing == 0 {
            Outcome::Rows { len: 2 }
        } else {
            Outcome::Truncated {
                len: 2,
                unread_rows: missing,
            }
        });
        super::decode_naval_payload(&mut captured, kit::BASE, &job, &mut [0; NAVAL_PACK_N]);
        assert_eq!(
            captured.wing_sources.outcome,
            Outcome::Truncated {
                len: 1,
                unread_rows: missing + 1
            }
        );
        assert_eq!(captured.wing_sources.rows.len(), 1);
    }
}

#[test]
fn shared_wing_fallback_charges_acquisition_and_installation() {
    use crate::extended::air::{AirWingCopyContext, AirWingJob, wing_test_kit as kit};
    let bytes = kit::fixture();
    let memory = kit::memory(&bytes);
    let mut wings = AirWingJob::new();
    let context = AirWingCopyContext {
        hour: 0,
        force_full: true,
        base: kit::BASE,
        game_state: kit::GAME_STATE,

        limits: [2, 3, 2],
        naval_limit: 0,
    };
    assert!(crate::extended::air::copy_air_refresh(&memory, context, &mut wings).raw_ok);
    wings.membership_ticks = 101;
    let mut captured = small();
    captured.add_wing_source_ticks(17);
    assert_eq!(captured.source_ticks[1], 17);
    captured.install_map_wings(&wings);
    assert!(captured.source_ticks[1] >= 118);
    assert_eq!(captured.wing_sources.outcome, Outcome::Rows { len: 2 });
    let first_ticks = captured.source_ticks[1];
    captured.add_wing_source_ticks(23);
    assert_eq!(captured.source_ticks[1], first_ticks + 23);
}

use engine::memory::SliceMemory;

fn small() -> Captured {
    Captured::with_capacities(&Capacities {
        rows: [2; N],
        theatre_names: 32,
        sunk_names: 32,
        orders_sources: 2,
        wing_sources: 2,
        roster_scratch: 8,
    })
}

#[test]
fn primary_membership_rejects_omissions_duplicates_and_incomplete_censuses() {
    let link = |row: &(u64, u64)| *row;
    let first_id = 0x1234_5678_0000_0035;
    let second_id = 0x9abc_def0_0000_0035;
    let group = 0xfedc_ba98_0000_004e;
    let rows = [(second_id, group), (first_id, group)];
    let sources = [(first_id, group), (42, 0), (second_id, group)];
    let full = Outcome::Rows { len: 2 };
    let census = Outcome::Rows { len: 3 };
    assert!(membership_issue(full, &rows, link, census, &sources, link).is_none());
    for bad in [
        vec![(first_id, group)],
        vec![(first_id, group), (first_id, group)],
        vec![(first_id, group), (second_id, group ^ (1_u64 << 32))],
    ] {
        let outcome = Outcome::Rows { len: bad.len() };
        assert!(membership_issue(outcome, &bad, link, census, &sources, link).is_some());
    }
    let partial = Outcome::Truncated {
        len: 2,
        unread_rows: 1,
    };
    assert!(membership_issue(full, &rows, link, partial, &sources, link).is_some());
    assert!(
        membership_issue(full, &rows, link, Outcome::Rows { len: 4 }, &sources, link).is_some()
    );
}

#[test]
fn wing_visitor_requires_a_complete_enclosing_census_and_preserves_full_handles() {
    let mut captured = small();
    captured.reset();
    let mut bytes = [0; 0x48];
    let group = 0x89ab_cdef_0000_004e_u64;
    bytes[cair_wing::GROUP_HANDLE..cair_wing::GROUP_HANDLE + 8]
        .copy_from_slice(&group.to_le_bytes());
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    let wing = cair_wing::Row {
        id: 0xfedc_ba98_0000_0009,
        ..Default::default()
    };
    captured.visit_wing(&memory, 0x1000, &wing);
    assert!(!complete_source(
        &captured.wing_sources,
        captured.wings_finished
    ));
    captured.finish_wings(Outcome::Rows { len: 1 });
    assert!(complete_source(
        &captured.wing_sources,
        captured.wings_finished
    ));
    assert_eq!(
        captured.wing_sources.rows[0],
        ctheatre::WingMembership {
            wing: wing.id,
            group
        }
    );
    captured.reset();
    captured.visit_wing(&memory, 0x1000, &wing);
    captured.finish_wings(Outcome::Truncated {
        len: 1,
        unread_rows: 2,
    });
    assert!(!complete_source(
        &captured.wing_sources,
        captured.wings_finished
    ));
    captured.reset();
    captured.visit_wing(&memory, 0x1000, &wing);
    captured.finish_wings(Outcome::Rows { len: 2 });
    assert!(matches!(
        captured.wing_sources.outcome,
        Outcome::Unavailable { .. }
    ));
}

#[test]
fn primary_name_ranges_preserve_bytes_and_refuse_invalid_extents() {
    let mut rows = CaptureRows::<ctheater_group::Row>::new(1);
    rows.outcome = Outcome::Rows { len: 1 };
    rows.rows[0] = ctheater_group::Row {
        id: u64::MAX,
        priority: -1,
        name_start: 0,
        name_len: 2,
        ..Default::default()
    };
    let name = |row: &ctheater_group::Row| (row.name_start, row.name_len);
    assert!(valid_named(rows.outcome, &rows.rows, Some(b"ab"), name));
    assert!(!valid_named(rows.outcome, &rows.rows, Some(b"a"), name));
    rows.rows[0].name_start = usize::MAX;
    assert!(!valid_named(rows.outcome, &rows.rows, Some(b"ab"), name));
}

#[test]
fn sunk_history_preserves_duplicate_records_and_never_publishes_a_prefix() {
    let mut captured = small();
    captured.sunk.outcome = Outcome::Rows { len: 2 };
    captured.sunk_names.first[..2].copy_from_slice(&[0xff, 0]);
    captured.sunk_names.len = 2;
    captured.sunk_names.required = 2;
    captured.sunk.rows[0].ship_name = csunk_ship_info::TextRange { start: 0, len: 2 };
    captured.sunk.rows[0].victim_equipment_variant = u64::MAX;
    captured.sunk.rows[1] = captured.sunk.rows[0];
    let TableView::Complete(TableRows::SunkShips(rows)) = captured.sunk_table() else {
        panic!("complete sequence expected");
    };
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].entries.len(), 2);
    assert_eq!(rows[0].entries[0], rows[0].entries[1]);
    assert_eq!(rows[0].entries[0].ship_name, [0xff, 0]);
    captured.sunk.outcome = Outcome::Truncated {
        len: 2,
        unread_rows: 1,
    };
    assert_eq!(
        captured.outcomes()[13],
        Outcome::Truncated {
            len: 0,
            unread_rows: 1
        }
    );
    assert_eq!(
        captured.sunk_table(),
        TableView::Truncated {
            rows: TableRows::SunkShips(Vec::new()),
            missing: 1
        }
    );
    captured.sunk.outcome = Outcome::Rows { len: 2 };
    captured.sunk.rows[0].ship_name.start = usize::MAX;
    assert!(matches!(
        captured.sunk_table(),
        TableView::Unavailable { .. }
    ));
}

#[test]
fn hourly_land_theatre_headers_need_no_daily_details_or_wing_storage() {
    use super::naval_test_kit as kit;
    struct NoTheatreDetails<'a>(engine::memory::SliceMemory<'a>);
    impl Memory for NoTheatreDetails<'_> {
        fn copy(&self, address: usize, bytes: &mut [u8]) -> bool {
            assert_ne!(address, kit::THEATRE + ctheatre::THEATER_GROUP_DATA);
            assert_ne!(address, kit::C0 + engine::ccountry::AIR_THEATRES);
            self.0.copy(address, bytes)
        }
    }
    let bytes = kit::fixture();
    let expected = kit::direct(&kit::memory(&bytes));
    let memory = NoTheatreDetails(kit::memory(&bytes));
    let mut job = NavalJob::new();
    job.wing_sources = Vec::new();
    let mut captured = Captured::with_capacities(&Capacities {
        rows: [8; N],
        theatre_names: 0,
        sunk_names: 0,
        orders_sources: 0,
        wing_sources: 0,
        roster_scratch: 0,
    });
    for hour in (0..24).filter(|hour| *hour != 13) {
        job.clear();
        captured.prepare_hour(hour, false);
        let copy = copy_naval_refresh(
            &memory,
            NavalCopyContext {
                base: kit::BASE,
                game_state: kit::GAME_STATE,
                hour,
                force_full: false,
            },
            &mut job,
        );
        assert!(copy.raw_ok && !job.grow_for_next());
        assert!(copy.raw_bytes[0] > 0);
        assert_eq!(&copy.raw_bytes[1..6], &[0; 5]);
        let mut ticks = [0; NAVAL_PACK_N];
        decode_naval_payload(&mut captured, kit::BASE, &job, &mut ticks);
        assert_eq!(captured.land.outcome, expected.land_outcome);
        assert_eq!(
            &captured.land.rows[..captured.land.outcome.len()],
            &expected.land[..expected.land_outcome.len()]
        );
        assert_eq!(&captured.outcomes()[5..10], &[SKIPPED; 5]);
        assert_eq!(captured.wing_sources.outcome, SKIPPED);
        assert_eq!(captured.groups.outcome, expected.oob_groups_outcome);
        assert_eq!(captured.group_members.outcome, expected.oob_members_outcome);
        assert_eq!(
            captured.group_children.outcome,
            expected.oob_children_outcome
        );
        let len = captured.groups.outcome.len();
        assert_eq!(&captured.groups.rows[..len], &expected.oob_groups[..len]);
        captured.fail_naval_capture("worker failed");
        assert_eq!(
            captured.land.outcome,
            Outcome::Unavailable {
                reason: "worker failed"
            }
        );
        assert_eq!(&captured.outcomes()[5..10], &[SKIPPED; 5]);
        assert_eq!(
            captured.groups.outcome,
            Outcome::Unavailable {
                reason: "worker failed"
            }
        );
    }
}
/// The land theatre copy through the callback's authenticated roster
/// takes no country census span, and its decode gives the rows, ordinals
/// and outcomes of the census copy and of the typed walk with or without
/// the roster: clean, with an alias or a null-slot owner (whose link the
/// replay reads), bad owners, a torn census, and a fault in every header
/// span, on the daily slot and hourly. A snapshot without room falls back
/// to the census copy and asks to grow.
#[test]
fn roster_land_theatres_match_the_census_copy_and_the_typed_walk() {
    use super::naval_test_kit as kit;
    use engine::{ccountry, cgame_state, country_memory::Cache, memory::SliceMemory};
    struct Fault<'a> {
        memory: SliceMemory<'a>,
        fault: Option<usize>,
    }
    impl Memory for Fault<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if self
                .fault
                .is_some_and(|fault| (address..address + output.len()).contains(&fault))
            {
                return false;
            }
            self.memory.copy(address, output)
        }
    }
    fn put(bytes: &mut [u8], at: usize, value: &[u8]) {
        bytes[at..at + value.len()].copy_from_slice(value);
    }
    const CTAB: usize = 0x11000;
    const LTAB: usize = 0x12000;
    let owner = kit::THEATRE + ctheatre::OWNER;
    // A fourth, null country slot whose id links to the first country.
    let mut base = kit::fixture();
    put(
        &mut base,
        kit::GAME_STATE + cgame_state::COUNTRIES + 8,
        &[4, 0, 0, 0, 4, 0, 0, 0],
    );
    put(
        &mut base,
        kit::GAME_STATE + cgame_state::COUNTRY_LINKS + 8,
        &[4, 0, 0, 0, 4, 0, 0, 0],
    );
    put(&mut base, LTAB + 12, &1_u32.to_le_bytes());
    put(
        &mut base,
        0x40000 + engine::cstrategic_air_manager::COUNTRY_AIR + 8,
        &[4, 0, 0, 0, 4, 0, 0, 0],
    );
    let mut variants: Vec<(&str, Vec<u8>, Option<usize>)> = vec![("clean", base.clone(), None)];
    for (name, value) in [
        ("alias owner", 2_u32),
        ("null-slot owner", 3),
        ("zero owner", 0),
        ("owner past links", 9),
    ] {
        let mut bytes = base.clone();
        put(&mut bytes, owner, &value.to_le_bytes());
        variants.push((name, bytes, None));
    }
    let mut torn = base.clone();
    put(&mut torn, LTAB + 8, &0_u32.to_le_bytes());
    variants.push(("torn census", torn, None));
    let mut vtable = base.clone();
    vtable[kit::THEATRE] ^= 0x10;
    variants.push(("theatre vtable", vtable, None));
    for (offset, len) in ctheatre::HEADER_SPANS {
        for at in [kit::THEATRE + offset, kit::THEATRE + offset + len - 1] {
            variants.push(("header fault", base.clone(), Some(at)));
        }
    }
    for at in [
        kit::C0 + ccountry::THEATRES,
        0x36000,
        CTAB + 8,
        kit::C0 + ccountry::ID,
    ] {
        variants.push(("country fault", base.clone(), Some(at)));
    }
    let hourly = |memory: &dyn Fn(&mut NavalJob) -> NavalCopyReport, job: &mut NavalJob| {
        let copy = memory(job);
        assert!(copy.raw_ok);
        let mut captured = Captured::with_capacities(&Capacities::default());
        captured.prepare_hour(1, false);
        decode_naval_payload(&mut captured, kit::BASE, job, &mut [0; NAVAL_PACK_N]);
        let len = captured.land.outcome.len();
        (captured.land.outcome, captured.land.rows[..len].to_vec())
    };
    let context = NavalCopyContext {
        base: kit::BASE,
        game_state: kit::GAME_STATE,
        hour: 1,
        force_full: false,
    };
    let mut roster_used = 0;
    for (name, bytes, fault) in &variants {
        let memory = Fault {
            memory: kit::memory(bytes),
            fault: *fault,
        };
        let mut cache = Cache::try_with_capacity(Cache::cells_for_countries(4, 64)).unwrap();
        let _ = cache.prepare(&memory, kit::BASE, kit::GAME_STATE);
        let roster = cache.roster_view(&memory);
        let has_roster = roster
            .authenticated_countries(kit::BASE, kit::GAME_STATE)
            .is_some();
        roster_used += usize::from(has_roster);
        let case = format!("{name} {fault:x?} roster={has_roster}");

        // Daily: every theatre table and the order of battle.
        let direct = kit::direct(&memory);
        kit::assert_matches_direct(&kit::direct(&roster), &direct);
        let (census, ..) = kit::split(&memory);
        kit::assert_matches_direct(&census, &direct);
        let (split, _, job) = kit::split(&roster);
        kit::assert_matches_direct(&split, &direct);
        assert_eq!(
            job.roster.roster(kit::BASE, kit::GAME_STATE).is_some(),
            has_roster,
            "{case}"
        );

        // Hourly: the land theatre headers alone.
        let mut plain = NavalJob::new();
        let expected = hourly(&|job| copy_naval_refresh(&memory, context, job), &mut plain);
        assert_eq!(
            (expected.0, &expected.1[..]),
            (
                direct.land_outcome,
                &direct.land[..direct.land_outcome.len()]
            ),
            "{case}"
        );
        let mut job = NavalJob::new();
        let actual = hourly(&|job| copy_naval_refresh(&roster, context, job), &mut job);
        assert_eq!(actual, expected, "{case}");
        if has_roster {
            // No census span: neither header, the country table, a
            // country's identity, nor a link the roster resolves.
            let spans: Vec<_> = job.log.spans.iter().map(|span| span.addr).collect();
            for address in [
                kit::GAME_STATE + cgame_state::COUNTRIES,
                kit::GAME_STATE + cgame_state::COUNTRY_LINKS,
                CTAB + 8,
                CTAB + 16,
                kit::C0,
                kit::C0 + ccountry::ID,
                LTAB + 4,
                LTAB + 8,
            ] {
                assert!(!spans.contains(&address), "{case}: {address:#x}");
            }
            assert!(spans.len() < plain.log.spans.len(), "{case}");
            // The replay reads the null slot's link, so the copy took it.
            if *name == "null-slot owner" {
                assert!(spans.contains(&(LTAB + 12)), "{case}");
            }
            // Without room for the roster the census copy is taken.
            let mut tight = NavalJob::new();
            tight.roster = ccountry::RosterSnapshot::try_with_capacity(0).unwrap();
            let short = hourly(&|job| copy_naval_refresh(&roster, context, job), &mut tight);
            assert_eq!(short, expected, "{case}");
            assert!(tight.roster.roster(kit::BASE, kit::GAME_STATE).is_none());
            assert!(tight.grow_for_next());
            assert_eq!(tight.log.spans.len(), plain.log.spans.len(), "{case}");
        }
    }
    assert!(roster_used >= variants.len() - 3, "{roster_used}");
}

#[test]
fn hourly_land_theatre_owner_fault_does_not_publish_a_header() {
    use super::naval_test_kit as kit;
    let mut bytes = kit::fixture();
    bytes[kit::THEATRE + ctheatre::OWNER..kit::THEATRE + ctheatre::OWNER + 4].fill(0);
    let memory = kit::memory(&bytes);
    let mut job = NavalJob::new();
    let copy = copy_naval_refresh(
        &memory,
        NavalCopyContext {
            base: kit::BASE,
            game_state: kit::GAME_STATE,
            hour: 1,
            force_full: false,
        },
        &mut job,
    );
    assert!(copy.raw_ok);
    let mut captured = Captured::with_capacities(&Capacities::default());
    captured.prepare_hour(1, false);
    decode_naval_payload(&mut captured, kit::BASE, &job, &mut [0; NAVAL_PACK_N]);
    assert!(matches!(captured.land.outcome, Outcome::Unavailable { .. }));
    assert_eq!(&captured.outcomes()[5..10], &[SKIPPED; 5]);
}
#[test]
fn land_theatre_headers_and_order_of_battle_are_hourly() {
    use replay::rows::TableId;
    for (slot, table) in [
        TableId::LandTheatres,
        TableId::LandTheatreGroups,
        TableId::LandTheatreMembers,
        TableId::AirTheatres,
        TableId::AirTheatreGroups,
        TableId::AirTheatreMembers,
        TableId::OrdersGroups,
        TableId::OrdersGroupMembers,
        TableId::ArmyGroupMembers,
    ]
    .into_iter()
    .enumerate()
    {
        let daily = (1..6).contains(&slot);
        assert_eq!(replay::cadence::is_staggered(table), daily, "{table:?}");
        for hour in -24_i32..48 {
            assert_eq!(
                replay::cadence::is_refresh(table, hour),
                !daily || hour.rem_euclid(24) == 13,
                "{table:?} at {hour}"
            );
        }
    }
}

#[test]
fn supremacy_and_sunk_offslot_jobs_skip_reads_and_decoding() {
    struct NoReads;
    impl Memory for NoReads {
        fn copy(&self, address: usize, _: &mut [u8]) -> bool {
            panic!("inactive naval read at {address:#x}");
        }
    }
    let mut supremacy = SupremacyJob::default();
    let mut sunk = SunkJob::default();
    let mut captured = owned_failure_fixture();
    for hour in 0..24 {
        captured.prepare_hour(hour, false);
        if hour != 23 {
            let report = copy_supremacy_refresh(
                &NoReads,
                SupremacyCopyContext {
                    base: 0,
                    game_state: 1,
                    hour,
                    force_full: false,
                },
                &mut supremacy,
            );
            assert!(!report.raw_ok && !supremacy.grow_for_next());
            assert_eq!((report.copy_ticks, report.raw_bytes), ([0], [0]));
            captured.fail_supremacy_capture("worker failed");
            let mut ticks = [0];
            captured.decode_supremacy_payload(0, &supremacy, &mut ticks);
            assert_eq!((captured.supremacy.outcome, ticks), (SKIPPED, [0]));
        }
        if hour != 19 {
            let report = copy_sunk_refresh(
                &NoReads,
                SunkCopyContext {
                    base: 0,
                    game_state: 1,
                    hour,
                    force_full: false,
                },
                &mut sunk,
            );
            assert!(!report.raw_ok && !sunk.grow_for_next());
            assert_eq!((report.copy_ticks, report.raw_bytes), ([0], [0]));
            captured.fail_sunk_capture("worker failed");
            let mut ticks = [0];
            captured.decode_sunk_payload(0, &sunk, &mut ticks);
            assert_eq!((captured.sunk.outcome, ticks), (SKIPPED, [0]));
        }
    }
}

#[test]
fn naval_split_copy_matches_direct_walk() {
    use super::naval_test_kit as kit;
    let bytes = kit::fixture();
    let memory = kit::memory(&bytes);
    let direct = kit::direct(&memory);
    let (split, report, _) = kit::split(&memory);
    assert!(report.raw_ok);
    kit::assert_matches_direct(&split, &direct);
    assert_eq!(split.land_outcome, Outcome::Rows { len: 1 });
    assert_eq!(split.members_outcome, Outcome::Rows { len: 2 });
    assert_eq!(split.groups_outcome, Outcome::Rows { len: 1 });
}

#[test]
fn naval_copy_sizes_cover_documented_read_set() {
    // S3: every Memory::copy size the naval copy pass issues, recorded
    // through the real pass. Registry headers (16/32), words (4/8),
    // theatre names (inline/heap) and OOB member rows. Alignment follows
    // the engine layout (headers 8-aligned, strings byte-packed).
    use super::naval_test_kit as kit;
    use super::{NavalCopyContext, NavalJob};
    use engine::Memory;
    use std::collections::BTreeSet;
    struct Recorder<'a> {
        inner: engine::memory::SliceMemory<'a>,
        sizes: std::cell::RefCell<BTreeSet<usize>>,
    }
    impl Memory for Recorder<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.sizes.borrow_mut().insert(output.len());
            self.inner.copy(address, output)
        }
    }
    let bytes = kit::fixture();
    let inner = kit::memory(&bytes);
    let memory = Recorder {
        inner,
        sizes: std::cell::RefCell::new(BTreeSet::new()),
    };
    let ctx = NavalCopyContext {
        hour: 0,
        force_full: true,
        base: kit::BASE,
        game_state: kit::GAME_STATE,
    };
    let mut first = NavalJob::new();
    kit::stage_wings(&memory, &mut first);
    let report = super::copy_naval_refresh(&memory, ctx, &mut first);
    assert!(report.raw_ok);
    let sizes = memory.sizes.borrow().clone();
    for required in [4, 8, 16] {
        assert!(
            sizes.contains(&required),
            "missing size {required} in {sizes:?}"
        );
    }
}

#[test]
fn naval_truncation_matrix_matches_direct_with_same_caps() {
    // B3: zero/one/exact-fit/overflow-by-one/massive run typed AND
    // copy→decode against the SAME memory with the SAME pooled caps,
    // asserting identical outcome (incl. len/unread) and identical rows.
    // Fixture holds 1 land theatre, 1 group, 2 members, 1 OOB group;
    // cap 1 is the overflow-by-one arm for members (total 2 == cap+1),
    // cap 0 the zero/massive arm, cap 2 the members exact-fit arm.
    // u32 unread saturation is not reachable and is covered by the
    // walk's own saturating arithmetic.
    use super::Capacities;
    use super::naval_test_kit as kit;
    let bytes = kit::fixture();
    let memory = kit::memory(&bytes);
    for rows in [[0; 17], [1; 17], [2; 17], [64; 17]] {
        let capacity = Capacities {
            rows,
            theatre_names: 1024,
            sunk_names: 1024,
            orders_sources: 64,
            wing_sources: 64,
            roster_scratch: 64,
        };
        let direct = kit::direct_with(&memory, &capacity);
        let (split, report, _) = kit::split_with(&memory, &capacity);
        assert!(report.raw_ok, "rows={rows:?}");
        kit::assert_matches_direct(&split, &direct);
    }
    // Exact-fit arm pins Rows on land/groups with both members present.
    let capacity = Capacities {
        rows: [2; 17],
        theatre_names: 1024,
        sunk_names: 1024,
        orders_sources: 64,
        wing_sources: 64,
        roster_scratch: 64,
    };
    let (split, _, _) = kit::split_with(&memory, &capacity);
    assert_eq!(split.land_outcome, Outcome::Rows { len: 1 });
    assert_eq!(split.members_outcome, Outcome::Rows { len: 2 });
}

#[test]
fn naval_split_copy_matches_direct_walk_under_truncation() {
    use super::naval_test_kit as kit;
    let bytes = kit::fixture();
    let memory = kit::memory(&bytes);
    let direct = kit::direct_truncated(&memory);
    let (split, report, _) = kit::split_truncated(&memory);
    assert!(report.raw_ok);
    kit::assert_matches_direct(&split, &direct);
}

#[test]
fn naval_split_copy_matches_direct_walk_on_fault_matrix() {
    use super::naval_test_kit as kit;
    use engine::memory::SliceMemory;
    struct Fault<'a> {
        memory: SliceMemory<'a>,
        fault: std::ops::Range<usize>,
    }
    impl engine::Memory for Fault<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            let Some(end) = address.checked_add(output.len()) else {
                return false;
            };
            if address < self.fault.end && end > self.fault.start {
                return false;
            }
            self.memory.copy(address, output)
        }
    }
    for fault in [
        kit::GAME_STATE + engine::cgame_state::COUNTRIES,
        kit::THEATRE,
        kit::GROUP,
    ] {
        let bytes = kit::fixture();
        let memory = Fault {
            memory: SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            fault: fault..fault + 1,
        };
        let direct = kit::direct(&memory);
        let (split, report, _) = kit::split(&memory);
        assert!(report.raw_ok, "fault={fault:x}");
        kit::assert_matches_direct(&split, &direct);
    }
}

#[test]
fn naval_staging_overflow_falls_back_to_typed_walk() {
    use super::naval_test_kit as kit;
    use super::{NavalCopyContext, NavalJob};
    let bytes = kit::fixture();
    let memory = kit::memory(&bytes);
    let ctx = NavalCopyContext {
        hour: 0,
        force_full: true,
        base: kit::BASE,
        game_state: kit::GAME_STATE,
    };
    let mut tiny = NavalJob {
        log: crate::raw::RawLog::new(1, 1),
        game_state: 0,
        ..Default::default()
    };
    kit::stage_wings(&memory, &mut tiny);
    let report = super::copy_naval_refresh(&memory, ctx, &mut tiny);
    assert!(!report.raw_ok);
}

#[test]
fn rail_tables_are_hourly_tier_h_with_no_refresh_gate() {
    use replay::rows::TableId;
    for table in [TableId::RailCooldowns, TableId::RailEdges] {
        assert!(!replay::cadence::is_staggered(table), "{table:?}");
        for hour in [0, 1, 2, 3, 60_820_531] {
            assert!(
                replay::cadence::is_refresh(table, hour),
                "{table:?} at {hour}"
            );
        }
    }
}

#[test]
fn supremacy_uses_slot_23() {
    use replay::rows::TableId;
    assert!(replay::cadence::is_staggered(TableId::NavalSupremacy));
    for hour in -24_i32..48 {
        assert_eq!(
            replay::cadence::is_refresh(TableId::NavalSupremacy, hour),
            hour.rem_euclid(24) == 23
        );
    }
}

#[test]
fn supremacy_split_copy_matches_direct_walk() {
    use super::supremacy_test_kit as kit;
    use engine::memory::{CountedMemory, ReadCounts};
    let bytes = kit::fixture();
    let memory = kit::memory(&bytes);
    let direct = kit::direct(&memory);
    let memory = CountedMemory::new(memory);
    let capture = read_accounting::Capture::new();
    let (split, report, mut job) = kit::split(&memory);
    let counts = capture.finish();
    assert!(report.raw_ok);
    kit::assert_matches_direct(&split, &direct);
    assert_eq!(split.outcome, Outcome::Rows { len: 2 });
    assert_eq!(split.rows[0].country, u32::MAX);
    assert_eq!(split.rows[0].current, 1000);
    assert_eq!(split.rows[1].country, 1);
    let anchor = TableId::NavalSupremacy;
    let mut expected = read_accounting::ZERO;
    // Header, two slots, three region fields, owner, entries header,
    // and one 160-byte value vector: nine scalar requests, 260 bytes.
    expected[anchor.index()] = ReadCounts {
        copy_calls: 9,
        spans: 9,
        bytes: 260,
        ..ReadCounts::ZERO
    };
    assert_eq!(counts, expected);
    let mut refreshes = 0;
    for hour in 0..24 {
        if replay::cadence::is_refresh(anchor, hour) {
            refreshes += 1;
            continue;
        }
        let capture = read_accounting::Capture::new();
        let skipped = copy_supremacy_refresh(
            &memory,
            SupremacyCopyContext {
                base: kit::BASE,
                game_state: kit::GAME_STATE,
                hour,
                force_full: false,
            },
            &mut job,
        );
        assert_eq!(capture.finish(), read_accounting::ZERO);
        assert!(!skipped.raw_ok && !job.grow_for_next());
        assert_eq!((skipped.copy_ticks, skipped.raw_bytes), ([0], [0]));
    }
    assert_eq!(refreshes, 1);
    let measured = counts[anchor.index()];
    let per_refresh = [
        measured.copy_calls,
        measured.scattered_calls,
        measured.record_calls,
        measured.plan_calls,
        measured.spans,
        measured.bytes,
    ];
    println!(
        "daily_fixture cohort=supremacy_split_copy_matches_direct_walk anchor={anchor:?} counters=[copy,scattered,records,plans,spans,bytes] per_refresh={per_refresh:?} before_24h={:?} after_24h={per_refresh:?} basis=unchanged_refresh_x_24_legacy_hourly_vs_1_daily",
        per_refresh.map(|value| value * 24)
    );
}

#[test]
fn supremacy_truncation_matrix_matches_direct_with_same_caps() {
    // B3: zero/one/exact-fit(2)/overflow-by-one/massive run typed AND
    // copy→decode against the SAME memory with the SAME pooled caps,
    // asserting identical outcome (incl. len/unread) and identical rows.
    // The bulk value vector serves every truncated prefix identically.
    use super::supremacy_test_kit as kit;
    let bytes = kit::fixture();
    let memory = kit::memory(&bytes);
    for cap in [0, 1, 2, 64] {
        let direct = kit::direct_with(&memory, cap);
        let (split, report, _) = kit::split_with(&memory, cap);
        assert!(report.raw_ok, "cap={cap}");
        kit::assert_matches_direct(&split, &direct);
    }
    let direct = kit::direct_with(&memory, 2);
    assert_eq!(direct.outcome, Outcome::Rows { len: 2 });
}

#[test]
fn supremacy_split_copy_matches_direct_walk_on_fault_matrix() {
    use super::supremacy_test_kit as kit;
    use engine::memory::SliceMemory;
    struct Fault<'a> {
        memory: SliceMemory<'a>,
        fault: std::ops::Range<usize>,
    }
    impl engine::Memory for Fault<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            let Some(end) = address.checked_add(output.len()) else {
                return false;
            };
            if address < self.fault.end && end > self.fault.start {
                return false;
            }
            self.memory.copy(address, output)
        }
    }
    for fault in [
        kit::GAME_STATE + engine::cgame_state::STRATEGIC_REGIONS,
        kit::REGION_OBJECT,
        kit::DOMINANCE,
        kit::DOMINANCE + engine::cnaval_region_dominance::ENTRIES,
    ] {
        let bytes = kit::fixture();
        let memory = Fault {
            memory: SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            fault: fault..fault + 1,
        };
        let direct = kit::direct(&memory);
        let (split, report, _) = kit::split(&memory);
        assert!(report.raw_ok, "fault={fault:x}");
        kit::assert_matches_direct(&split, &direct);
    }
}

#[test]
fn supremacy_staging_overflow_falls_back_to_typed_walk() {
    use super::supremacy_test_kit as kit;
    use super::{SupremacyCopyContext, SupremacyJob};
    let bytes = kit::fixture();
    let memory = kit::memory(&bytes);
    let ctx = SupremacyCopyContext {
        hour: 0,
        force_full: true,
        base: kit::BASE,
        game_state: kit::GAME_STATE,
    };
    let mut tiny = SupremacyJob {
        log: crate::raw::RawLog::new(1, 1),
        game_state: 0,
        ..Default::default()
    };
    let report = super::copy_supremacy_refresh(&memory, ctx, &mut tiny);
    assert!(!report.raw_ok);
}

#[test]
fn supremacy_pools_cover_appendix_worst_case() {
    // Worst-case caps proof: the steady-state pools exceed a
    // conservative span/byte estimate built from the appendix census (49
    // supremacy rows over a handful of regions), so live refreshes never
    // take the overflow fallback in steady state.
    use super::SupremacyJob;
    let job = SupremacyJob::new();
    let spans = 1 + 10 * 6 + 49 * 2;
    assert!(
        job.log.spans.capacity() >= spans,
        "span cap {} < worst case {spans}",
        job.log.spans.capacity()
    );
    let bytes = 10 * 64 + 49 * 128;
    assert!(
        job.log.bytes.capacity() >= bytes,
        "byte cap {} < worst case {bytes}",
        job.log.bytes.capacity()
    );
}

#[test]
fn supremacy_copy_sizes_cover_documented_read_set() {
    // S3: every Memory::copy size the supremacy copy pass issues,
    // recorded through the real pass. Roster/identity words (8/12/16)
    // and bulk value vectors (whole-vector takes, served in 32-entry
    // chunks at replay).
    use super::supremacy_test_kit as kit;
    use engine::Memory;
    use std::collections::BTreeSet;
    struct Recorder<'a> {
        inner: engine::memory::SliceMemory<'a>,
        sizes: std::cell::RefCell<BTreeSet<usize>>,
    }
    impl Memory for Recorder<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.sizes.borrow_mut().insert(output.len());
            self.inner.copy(address, output)
        }
    }
    let bytes = kit::fixture();
    let memory = Recorder {
        inner: kit::memory(&bytes),
        sizes: std::cell::RefCell::new(BTreeSet::new()),
    };
    let (_split, report, _) = kit::split(&memory);
    assert!(report.raw_ok);
    let sizes = memory.sizes.borrow().clone();
    for required in [8, 12, 16] {
        assert!(
            sizes.contains(&required),
            "missing size {required} in {sizes:?}"
        );
    }
}

#[test]
fn sunk_ships_use_slot_19() {
    use replay::rows::TableId;
    assert!(replay::cadence::is_staggered(TableId::SunkShips));
    for hour in -24_i32..48 {
        assert_eq!(
            replay::cadence::is_refresh(TableId::SunkShips, hour),
            hour.rem_euclid(24) == 19
        );
    }
}

#[test]
fn sunk_split_copy_matches_direct_walk() {
    use super::sunk_test_kit as kit;
    use engine::memory::{CountedMemory, ReadCounts};
    let bytes = kit::fixture();
    let memory = kit::memory(&bytes);
    let direct = kit::direct(&memory);
    let memory = CountedMemory::new(memory);
    let capture = read_accounting::Capture::new();
    let (split, report, mut job) = kit::split(&memory);
    let counts = capture.finish();
    assert!(report.raw_ok);
    kit::assert_matches_direct(&split, &direct);
    // Two retained records: one with attribution class and province,
    // one minimal with a long inline name and heap attribution name.
    assert_eq!(split.outcome, Outcome::Rows { len: 2 });
    assert_eq!(split.rows[0].victim_country, 3);
    assert_eq!(split.rows[0].province, 77);
    assert_eq!(split.rows[0].attribution_class_present, 1);
    assert_eq!(split.rows[1].victim_country, 5);
    assert_eq!(split.rows[1].province_present, 0);
    assert_eq!(split.rows[1].attribution_class_present, 0);
    let anchor = TableId::SunkShips;
    let mut expected = read_accounting::ZERO;
    // Header + two slots/records + three class headers + seven strings
    // (terminator and content each) + province identity: 24 requests, 585 bytes.
    expected[anchor.index()] = ReadCounts {
        copy_calls: 24,
        spans: 24,
        bytes: 585,
        ..ReadCounts::ZERO
    };
    assert_eq!(counts, expected);
    let mut refreshes = 0;
    for hour in kit::HOUR..kit::HOUR + 24 {
        if replay::cadence::is_refresh(anchor, hour) {
            refreshes += 1;
            continue;
        }
        let capture = read_accounting::Capture::new();
        let skipped = copy_sunk_refresh(
            &memory,
            SunkCopyContext {
                base: kit::BASE,
                game_state: kit::GAME_STATE,
                hour,
                force_full: false,
            },
            &mut job,
        );
        assert_eq!(capture.finish(), read_accounting::ZERO);
        assert!(!skipped.raw_ok && !job.grow_for_next());
        assert_eq!((skipped.copy_ticks, skipped.raw_bytes), ([0], [0]));
    }
    assert_eq!(refreshes, 1);
    let measured = counts[anchor.index()];
    let per_refresh = [
        measured.copy_calls,
        measured.scattered_calls,
        measured.record_calls,
        measured.plan_calls,
        measured.spans,
        measured.bytes,
    ];
    println!(
        "daily_fixture cohort=sunk_split_copy_matches_direct_walk anchor={anchor:?} counters=[copy,scattered,records,plans,spans,bytes] per_refresh={per_refresh:?} before_24h={:?} after_24h={per_refresh:?} basis=unchanged_refresh_x_24_legacy_hourly_vs_1_daily",
        per_refresh.map(|value| value * 24)
    );
}

#[test]
fn sunk_truncation_matrix_matches_direct_with_same_caps() {
    // B3: zero/one/exact-fit(2)/overflow-by-one/massive run typed AND
    // copy→decode against the SAME memory with the SAME pooled caps,
    // asserting identical outcome (incl. len/unread) and identical rows.
    use super::sunk_test_kit as kit;
    let bytes = kit::fixture();
    let memory = kit::memory(&bytes);
    for (rows, names) in [(0, 0), (1, 16), (2, 64), (64, 4096)] {
        let direct = kit::direct_with(&memory, rows, names);
        let (split, report, _) = kit::split_with(&memory, rows, names);
        assert!(report.raw_ok, "rows={rows} names={names}");
        kit::assert_matches_direct(&split, &direct);
    }
    let direct = kit::direct_with(&memory, 2, 64);
    assert_eq!(direct.outcome, Outcome::Rows { len: 2 });
}

#[test]
fn sunk_split_copy_matches_direct_walk_on_fault_matrix() {
    use super::sunk_test_kit as kit;
    use engine::memory::SliceMemory;
    struct Fault<'a> {
        memory: SliceMemory<'a>,
        fault: std::ops::Range<usize>,
    }
    impl engine::Memory for Fault<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            let Some(end) = address.checked_add(output.len()) else {
                return false;
            };
            if address < self.fault.end && end > self.fault.start {
                return false;
            }
            self.memory.copy(address, output)
        }
    }
    for fault in [
        kit::GAME_STATE + engine::cgame_state::SUNK_SHIP_HISTORY,
        kit::FIRST,
        kit::FIRST + engine::csunk_ship_info::SINK_HOUR,
        kit::CLASS,
        kit::FIRST + engine::csunk_ship_info::SHIP_NAME,
    ] {
        let bytes = kit::fixture();
        let memory = Fault {
            memory: SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            fault: fault..fault + 1,
        };
        let direct = kit::direct(&memory);
        let (split, report, _) = kit::split(&memory);
        assert!(report.raw_ok, "fault={fault:x}");
        kit::assert_matches_direct(&split, &direct);
    }
}

#[test]
fn sunk_staging_overflow_falls_back_to_typed_walk() {
    use super::sunk_test_kit as kit;
    use super::{SunkCopyContext, SunkJob};
    let bytes = kit::fixture();
    let memory = kit::memory(&bytes);
    let ctx = SunkCopyContext {
        force_full: true,
        base: kit::BASE,
        game_state: kit::GAME_STATE,
        hour: kit::HOUR,
    };
    let mut tiny = SunkJob {
        log: crate::raw::RawLog::new(1, 1),
        game_state: 0,
        hour: 0,
        ..Default::default()
    };
    let report = super::copy_sunk_refresh(&memory, ctx, &mut tiny);
    assert!(!report.raw_ok);
}

#[test]
fn sunk_pools_cover_appendix_worst_case() {
    // Worst-case caps proof: the steady-state pools exceed a
    // conservative span/byte estimate built from the appendix census
    // (one row with ~371 retained entries and names), so live refreshes
    // never take the overflow fallback in steady state.
    use super::SunkJob;
    let job = SunkJob::new();
    let spans = 1 + 371 * 12;
    assert!(
        job.log.spans.capacity() >= spans,
        "span cap {} < worst case {spans}",
        job.log.spans.capacity()
    );
    let bytes = 371 * 512;
    assert!(
        job.log.bytes.capacity() >= bytes,
        "byte cap {} < worst case {bytes}",
        job.log.bytes.capacity()
    );
}

#[test]
fn sunk_copy_sizes_cover_documented_read_set() {
    // S3: every Memory::copy size the sunk copy pass issues, recorded
    // through the real pass. Vector words (8/16), records (0xa8),
    // class headers (0x38), name bytes (1 + content) and province
    // words (8/4).
    use super::sunk_test_kit as kit;
    use engine::Memory;
    use std::collections::BTreeSet;
    struct Recorder<'a> {
        inner: engine::memory::SliceMemory<'a>,
        sizes: std::cell::RefCell<BTreeSet<usize>>,
    }
    impl Memory for Recorder<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.sizes.borrow_mut().insert(output.len());
            self.inner.copy(address, output)
        }
    }
    let bytes = kit::fixture();
    let memory = Recorder {
        inner: kit::memory(&bytes),
        sizes: std::cell::RefCell::new(BTreeSet::new()),
    };
    let (_split, report, _) = kit::split(&memory);
    assert!(report.raw_ok);
    let sizes = memory.sizes.borrow().clone();
    for required in [1, 4, 8, 16, 0xa8] {
        assert!(
            sizes.contains(&required),
            "missing size {required} in {sizes:?}"
        );
    }
}
