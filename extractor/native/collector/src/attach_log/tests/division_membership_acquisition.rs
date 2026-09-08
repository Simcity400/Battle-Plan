//! Actual root sampler fixtures: the reader delegates to owned process memory.
//! Faults are injected inside sample_division_movement_with_training_reader,
//! so its original frame, preflight, siblings and mandatory adapter close run.
use super::*;
use compact_contract::build_profile as profile;
use compact_contract::division_routes::{
    CombatClass, CombatIdentity, CombatMembershipCause, CombatMembershipField,
    CountryWordsObservation, FirstCombatCountriesObservation, TrainingObservation,
};
use division_combat_membership::{Cause, Field, Refusal, Stage};
use division_frame_authority::{AttachmentIdentity, CampaignBinding};
use division_presentation_inputs::MotionFrameInput;
use std::sync::atomic::{AtomicUsize, Ordering};

mod assembled_publication;

const UUID: [u8; 36] = *b"a4b17fed-ed12-4361-9064-fb6576638110";
const SEED: i32 = -1234;
const HOUR: i32 = compact_contract::calendar::DATE_EPOCH_CONST + 1936 * 365 * 24 + 100;
const COUNTRY: i32 = -7;
const COUNTRIES: [i32; 4] = [COUNTRY, 0, COUNTRY, i32::MIN];
const ATTACHMENT: AttachmentIdentity = AttachmentIdentity {
    pid: 17,
    creation_filetime: 29,
};

fn campaign() -> CampaignBinding<'static> {
    CampaignBinding {
        logical_stream_id: 1,
        game_unique_id: UUID,
        game_unique_seed: SEED,
        custody: None,
    }
}

fn combat_identity() -> CombatIdentity {
    CombatIdentity {
        class: CombatClass::Land,
        kind: LAND_COMBAT_TYPE,
        id: 79,
    }
}

struct Fixture {
    root_slot: Box<AlignedBytes<8>>,
    game_state: Box<AlignedBytes<0x800>>,
    uuid: Box<AlignedBytes<40>>,
    primary: Box<AlignedBytes<0x800>>,
    _stats: Box<AlignedBytes<0x300>>,
    _combat_vector: Box<AlignedBytes<8>>,
    combat: Box<AlignedBytes<0x80>>,
    side: Box<AlignedBytes<0x60>>,
    nodes: Box<AlignedBytes<0x80>>,
    module: usize,
}

impl Fixture {
    fn new() -> Self {
        let mut root_slot = AlignedBytes::new();
        let mut game_state = AlignedBytes::new();
        let mut uuid = AlignedBytes::new();
        let mut primary = AlignedBytes::new();
        let mut stats = AlignedBytes::new();
        let mut combat_vector = AlignedBytes::new();
        let mut combat = AlignedBytes::new();
        let mut side = AlignedBytes::new();
        let mut nodes = AlignedBytes::new();
        let module = root_slot
            .addr()
            .checked_sub(profile::DOMAIN57_CGAMESTATE_SINGLETON_RVA)
            .unwrap();
        let body = DIVISION_WRAPPER_TO_BODY;
        root_slot.write_u64(0, game_state.addr() as u64);
        game_state.write_i32(GS_CURRENT_DATE_OFFSET, HOUR);
        game_state.write_i32(profile::DOMAIN_CAMPAIGN_ID_GAME_UNIQUE_SEED_OFF, SEED);
        game_state.write_u64(
            profile::DOMAIN_CAMPAIGN_ID_GAME_UNIQUE_ID_OFF,
            uuid.addr() as u64,
        );
        game_state.write_u64(
            profile::DOMAIN_CAMPAIGN_ID_GAME_UNIQUE_ID_OFF + 0x10,
            UUID.len() as u64,
        );
        game_state.write_u64(
            profile::DOMAIN_CAMPAIGN_ID_GAME_UNIQUE_ID_OFF + 0x18,
            UUID.len() as u64,
        );
        uuid.write_bytes(0, &UUID);
        primary.write_u64(0, (module + DIVISION_WRAPPER_VTABLE_RVA) as u64);
        primary.write_u64(body, (module + DIVISION_VTABLE_RVA) as u64);
        primary.write_u32(body + DIVISION_OFF_ID_TYPE, DIVISION_TYPE);
        primary.write_u32(body + DIVISION_OFF_ID_ID, 7);
        primary.write_u32(body + DIVISION_OFF_LOCATION, 12);
        primary.write_u32(body + DIVISION_OFF_STRENGTH, 100_000);
        primary.write_u32(body + DIVISION_OFF_ORGANISATION, 200_000);
        primary.write_u32(body + DIVISION_OFF_OWNER_TAG_DB_ID, 3);
        primary.write_i32(0x1e0, COUNTRY);
        primary.write_u64(DIVISION_WRAPPER_OFF_COUNTER_STATS, stats.addr() as u64);
        stats.write_u64(0, (module + DIVISION_COUNTER_STATS_VTABLE_RVA) as u64);
        stats.write_u64(DIVISION_COUNTER_STATS_OFF_MAX_ORGANISATION, 300_000);
        stats.write_u64(0x288, 150_000);
        primary.write_u64(0x430, 42_000);
        primary.write_u64(body + 0x230, (-5i64) as u64);
        primary.write_u8(body + 0x23c, 255);
        primary.write_u64(body + 0x198, combat_vector.addr() as u64);
        primary.write_u32(body + 0x1a0, 1);
        primary.write_u32(body + DIVISION_OFF_COMBAT_REF_COUNT, 1);
        combat_vector.write_u64(0, combat.addr() as u64);
        combat.write_u64(0, (module + LAND_COMBAT_PRIMARY_VTABLE_RVA) as u64);
        combat.write_u64(
            LAND_COMBAT_MANAGER_BODY_DELTA,
            (module + LAND_COMBAT_BODY_VTABLE_RVA) as u64,
        );
        combat.write_u32(
            LAND_COMBAT_MANAGER_BODY_DELTA + LAND_COMBAT_OFF_TYPE,
            LAND_COMBAT_TYPE,
        );
        combat.write_u32(
            LAND_COMBAT_MANAGER_BODY_DELTA + LAND_COMBAT_OFF_ID,
            combat_identity().id,
        );
        combat.write_u64(0x28, side.addr() as u64);
        side.write_u64(0, (module + LAND_COMBAT_SIDE_VTABLE_RVA) as u64);
        side.write_u64(0x18, combat.addr() as u64);
        side.write_u64(0x38, nodes.addr() as u64);
        for (index, country) in COUNTRIES.iter().enumerate() {
            nodes.write_i32(index * 0x20, *country);
            let next = if index + 1 == COUNTRIES.len() {
                0
            } else {
                nodes.addr() + (index + 1) * 0x20
            };
            nodes.write_u64(index * 0x20 + 0x10, next as u64);
        }
        Self {
            root_slot,
            game_state,
            uuid,
            primary,
            _stats: stats,
            _combat_vector: combat_vector,
            combat,
            side,
            nodes,
            module,
        }
    }

    fn entry(&self) -> DivisionBodyBridgeEntry {
        DivisionBodyBridgeEntry {
            div_id: 7,
            body_ptr: self.primary.addr() + DIVISION_WRAPPER_TO_BODY,
            owner_tag_db_id: 3,
        }
    }

    fn frame(&self) -> MotionFrameInput {
        MotionFrameInput {
            game_state: self.game_state.addr(),
            raw_hour: HOUR,
        }
    }
}

#[derive(Default)]
struct Reads(BTreeMap<(usize, usize), usize>);

impl Reads {
    fn note(&mut self, at: usize, width: usize) -> usize {
        let count = self.0.entry((at, width)).or_default();
        *count += 1;
        *count
    }

    fn count(&self, at: usize, width: usize) -> usize {
        self.0.get(&(at, width)).copied().unwrap_or(0)
    }

    fn assert_drained(&self) {
        assert!(!self.0.is_empty());
        for ((at, width), count) in &self.0 {
            assert_eq!(
                count % 2,
                0,
                "unclosed original address {at:#x}/{width}: {count} visits"
            );
        }
    }
}

fn assert_measured(sample: &DivisionMovementSample) {
    assert_eq!(
        sample.training_scalars.country_operand,
        TrainingObservation::Measured(COUNTRY)
    );
    assert_eq!(
        sample.movement_scalars.first_combat,
        Some(Some(combat_identity()))
    );
    assert_eq!(
        sample.first_combat_countries,
        FirstCombatCountriesObservation::Present {
            combat: combat_identity(),
            countries: CountryWordsObservation::Measured(COUNTRIES.to_vec()),
        }
    );
}

#[test]
fn actual_sampler_pairs_signed_countries_and_reads_country_operand_only_once_each_way() {
    let _counters = lock_rc_counters();
    page_cache_new_frame();
    let fixture = Fixture::new();
    let mut reads = Reads::default();
    let mut attachment_reads = 0;
    let sample = sample_division_movement_with_training_reader(
        fixture.module,
        &fixture.entry(),
        fixture.frame(),
        Some(&campaign()),
        &country_roster_readset::Budget::default(),
        &mut |at, out| {
            reads.note(at, out.len());
            read_into(at, out)
        },
        &mut || {
            attachment_reads += 1;
            Some(ATTACHMENT)
        },
    )
    .unwrap()
    .expect("complete native acquisition produces its division");
    assert_measured(&sample);
    assert_eq!(sample.raw_strength, 100_000);
    assert_eq!(sample.raw_organisation, 200_000);
    assert_eq!(sample.movement_scalars.numerator_raw, Some(-5));
    assert_eq!(sample.movement_scalars.retreat_raw, Some(255));
    assert_eq!(reads.count(fixture.primary.addr() + 0x1e0, 4), 2);
    assert_eq!(reads.count(fixture._combat_vector.addr(), 8), 2);
    for index in 0..COUNTRIES.len() {
        assert_eq!(reads.count(fixture.nodes.addr() + index * 0x20, 4), 2);
        assert_eq!(
            reads.count(fixture.nodes.addr() + index * 0x20 + 0x10, 8),
            2
        );
    }
    assert_eq!(attachment_reads, 2);
    reads.assert_drained();
}

#[test]
fn actual_sampler_unreadable_strength_or_organisation_is_fatal_instead_of_dropping_row() {
    let _counters = lock_rc_counters();
    page_cache_new_frame();
    for (offset, field) in [
        (DIVISION_OFF_STRENGTH, Field::Strength),
        (DIVISION_OFF_ORGANISATION, Field::Organisation),
    ] {
        let fixture = Fixture::new();
        let failed = fixture.entry().body_ptr + offset;
        let mut reads = Reads::default();
        let result = sample_division_movement_with_training_reader(
            fixture.module,
            &fixture.entry(),
            fixture.frame(),
            Some(&campaign()),
            &country_roster_readset::Budget::default(),
            &mut |at, out| {
                let visit = reads.note(at, out.len());
                if at == failed && visit == 1 {
                    false
                } else {
                    read_into(at, out)
                }
            },
            &mut || Some(ATTACHMENT),
        );
        assert!(
            matches!(result, Err(DivisionSampleFailure::FrameAuthorityUnavailable(Refusal {
            cause: Cause::Unreadable, field: actual, stage: Stage::Open, ..
        })) if actual == field)
        );
        assert_eq!(reads.count(failed, 4), 2);
        assert_eq!(reads.count(fixture.primary.addr() + 0x1e0, 4), 0);
        reads.assert_drained();
    }
}

#[test]
fn actual_sampler_campaign_mutation_inside_side_walk_closes_original_frame_and_refuses_row() {
    let _counters = lock_rc_counters();
    page_cache_new_frame();
    let mut fixture = Fixture::new();
    let module = fixture.module;
    let entry = fixture.entry();
    let frame = fixture.frame();
    let side_head = fixture.side.addr() + 0x38;
    let mut reads = Reads::default();
    let result = sample_division_movement_with_training_reader(
        module,
        &entry,
        frame,
        Some(&campaign()),
        &country_roster_readset::Budget::default(),
        &mut |at, out| {
            let visit = reads.note(at, out.len());
            if at == side_head && visit == 1 {
                fixture.uuid.write_u8(0, b'b');
            }
            read_into(at, out)
        },
        &mut || Some(ATTACHMENT),
    );
    assert!(matches!(
        result,
        Err(DivisionSampleFailure::FrameAuthorityUnavailable(Refusal {
            cause: Cause::Changed,
            field: Field::CampaignIdentity,
            stage: Stage::Close,
            ..
        }))
    ));
    assert_eq!(reads.count(fixture.uuid.addr(), 8), 2);
    assert_eq!(reads.count(fixture.combat.addr(), 8), 2);
    assert_eq!(reads.count(side_head, 8), 2);
    for index in 0..COUNTRIES.len() {
        assert_eq!(reads.count(fixture.nodes.addr() + index * 0x20, 4), 2);
        assert_eq!(
            reads.count(fixture.nodes.addr() + index * 0x20 + 0x10, 8),
            2
        );
    }
    reads.assert_drained();
}

#[test]
fn actual_sampler_side_only_failure_preserves_sealed_first_combat_and_paired_country() {
    let _counters = lock_rc_counters();
    page_cache_new_frame();
    let fixture = Fixture::new();
    let mut reads = Reads::default();
    let sample = sample_division_movement_with_training_reader(
        fixture.module,
        &fixture.entry(),
        fixture.frame(),
        Some(&campaign()),
        &country_roster_readset::Budget::default(),
        &mut |at, out| {
            let visit = reads.note(at, out.len());
            if at == fixture.side.addr() && visit == 1 {
                false
            } else {
                read_into(at, out)
            }
        },
        &mut || Some(ATTACHMENT),
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        sample.training_scalars.country_operand,
        TrainingObservation::Measured(COUNTRY)
    );
    assert_eq!(
        sample.movement_scalars.first_combat,
        Some(Some(combat_identity()))
    );
    assert!(
        matches!(sample.first_combat_countries, FirstCombatCountriesObservation::Present {
        combat, countries: CountryWordsObservation::Unavailable(reason),
    } if combat == combat_identity() && reason.cause == CombatMembershipCause::Unreadable
        && reason.field == CombatMembershipField::SideClass)
    );
    assert_eq!(reads.count(fixture.primary.addr() + 0x1e0, 4), 2);
    assert_eq!(reads.count(fixture.side.addr(), 8), 2);
    assert_eq!(reads.count(fixture.nodes.addr(), 4), 0);
    reads.assert_drained();
}

#[test]
fn root_sampler_propagates_actual_acquisition_failure_without_publishing_successful_prefix() {
    let _counters = lock_rc_counters();
    page_cache_new_frame();
    let fixture = Fixture::new();
    let binding = campaign();
    let calls = AtomicUsize::new(0);
    let result = sample_compact_division_roots_with(
        fixture.module,
        &[fixture.entry(), fixture.entry()],
        &country_roster_readset::Budget::default(),
        |module, entry, budget| {
            let fail = calls.fetch_add(1, Ordering::Relaxed) == 1;
            let mut reads = Reads::default();
            let result = sample_division_movement_with_training_reader(
                module,
                entry,
                fixture.frame(),
                Some(&binding),
                budget,
                &mut |at, out| {
                    let visit = reads.note(at, out.len());
                    if fail && at == entry.body_ptr + DIVISION_OFF_ORGANISATION && visit == 1 {
                        false
                    } else {
                        read_into(at, out)
                    }
                },
                &mut || Some(ATTACHMENT),
            );
            reads.assert_drained();
            result
        },
    );
    assert_eq!(calls.load(Ordering::Relaxed), 2);
    assert!(matches!(
        result,
        Err(DivisionSampleFailure::FrameAuthorityUnavailable(Refusal {
            cause: Cause::Unreadable,
            field: Field::Organisation,
            ..
        }))
    ));
}

#[test]
fn root_sampler_zero_retained_budget_rejects_before_any_body_callback() {
    let fixture = Fixture::new();
    let calls = AtomicUsize::new(0);
    let budget = country_roster_readset::Budget::test_limits(0, 0);
    let result = sample_compact_division_roots_with(
        fixture.module,
        &[fixture.entry()],
        &budget,
        |_, _, _| {
            calls.fetch_add(1, Ordering::Relaxed);
            panic!("unfunded body callback");
        },
    );
    assert!(matches!(
        result,
        Err(DivisionSampleFailure::OutputStorageUnavailable)
    ));
    assert_eq!(calls.load(Ordering::Relaxed), 0);
    assert_eq!(budget.verification_bytes(), 0);
    assert_eq!(budget.retained_bytes(), 1);
}

#[test]
fn real_division_pipeline_keeps_baseline_on_root_fault_and_recovers_complete_membership() {
    let _counters = lock_rc_counters();
    page_cache_new_frame();
    let mut fixture = Fixture::new();
    let mut state = empty_collector_state();
    state.base = fixture.module;
    state.date_ptr = fixture.game_state.addr() as u64;
    let control = raw_test_control();
    let mut roots = CompactDivisionRootCache::default();
    roots.rows.push(fixture.entry());
    roots.complete = true;
    roots.observed_candidate_rows = 1;
    let mut tracker: FrameJournalled<BTreeMap<u32, DivisionMovementSample>> = FrameJournalled::default();
    tracker.advance(BTreeMap::from([(
        99,
        deleted_division_sample(99, DIVISION_TYPE),
    )]));
    tracker.close_frame_transaction();
    fixture
        .root_slot
        .write_u64(0, fixture.game_state.addr() as u64 + 8);
    let refused = read_compact_divisions(
        &state,
        &control,
        0,
        &mut tracker,
        &roots,
        &country_roster_readset::Budget::default(),
        HOUR,
        Some(&campaign()),
    );
    assert!(refused.rows.is_empty());
    assert_eq!(refused.changed_rows, 0);
    assert_eq!(refused.root_rows, 1);
    assert_ne!(refused.status_flags & COMPACT_STATUS_SOURCE_UNAVAILABLE, 0);
    assert!(matches!(refused.frame_identity_refusal, Some(reason)
        if reason.cause == CombatMembershipCause::RootChanged && reason.field == CombatMembershipField::FrameRoot));
    assert_eq!(tracker.len(), 1);
    assert!(tracker.get(&99).is_some_and(|row| row.deleted));
    assert!(
        !tracker.has_frame_entry(),
        "refused acquisition cannot stage a baseline advance"
    );
    stage_division_publication_baseline(&refused, &mut tracker);
    assert!(
        tracker.is_empty(),
        "publishing the gap stages an empty baseline"
    );
    tracker.close_frame_transaction();
    fixture
        .root_slot
        .write_u64(0, fixture.game_state.addr() as u64);
    let recovered = read_compact_divisions(
        &state,
        &control,
        0,
        &mut tracker,
        &roots,
        &country_roster_readset::Budget::default(),
        HOUR,
        Some(&campaign()),
    );
    assert_eq!(recovered.frame_identity_refusal, None);
    assert!(!recovered.output_storage_unavailable);
    assert!(!compact_contract::compact_status_is_failure(
        recovered.status_flags
    ));
    assert_eq!(recovered.status_flags & COMPACT_STATUS_DELTA_DATA, 0);
    assert_eq!(recovered.root_rows, 1);
    assert_eq!(recovered.changed_rows, 1);
    assert_eq!(
        recovered.rows.len(),
        1,
        "recovery is a complete full baseline"
    );
    let division = recovered
        .rows
        .iter()
        .find(|row| row.div_id == 7 && !row.deleted)
        .expect("recovery emits the complete acquired division");
    assert_measured(division);
    assert_eq!(tracker.len(), 1);
    assert_measured(
        tracker
            .get(&7)
            .expect("recovered baseline owns the complete list"),
    );
    stage_division_publication_baseline(&recovered, &mut tracker);
    tracker.close_frame_transaction();

    let unchanged = read_compact_divisions(
        &state,
        &control,
        0,
        &mut tracker,
        &roots,
        &country_roster_readset::Budget::default(),
        HOUR,
        Some(&campaign()),
    );
    assert_eq!(unchanged.frame_identity_refusal, None);
    assert!(!unchanged.output_storage_unavailable);
    assert!(!compact_contract::compact_status_is_failure(
        unchanged.status_flags
    ));
    assert_ne!(unchanged.status_flags & COMPACT_STATUS_DELTA_DATA, 0);
    assert_eq!(unchanged.root_rows, 1);
    assert_eq!(unchanged.changed_rows, 0);
    assert!(
        unchanged.rows.is_empty(),
        "unchanged acquired membership emits an empty delta"
    );
    assert_measured(
        tracker
            .get(&7)
            .expect("empty delta retains the recovered baseline"),
    );
}
