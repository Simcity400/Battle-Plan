//! Synthetic owned memory through the production D20 root sampler and publisher.
//! These fixtures establish acquisition/transport behavior, not live-game admission.
use super::*;
use compact_contract::build_profile as profile;
use compact_contract::division_routes::{
    self as wire, CombatMembershipCause as Cause, CombatMembershipStage as Stage,
    DivisionOwnerAuthority as Owner, RetainedVisualVariantField as Field,
    RetainedVisualVariantObservation as Variant, RetainedVisualVariantRefusal as Refusal,
};
use country_roster_readset::Budget;
use division_frame_authority::{AttachmentIdentity, CampaignBinding};
use division_presentation_inputs::MotionFrameInput;
use std::cell::{Cell, RefCell};

const UUID: [u8; 36] = *b"a4b17fed-ed12-4361-9064-fb6576638110";
const SEED: i32 = -1234;
const HOUR: i32 = compact_contract::calendar::DATE_EPOCH_CONST + 1936 * 365 * 24 + 100;
const ATTACHMENT: AttachmentIdentity = AttachmentIdentity {
    pid: 17,
    creation_filetime: 29,
};

#[repr(align(8))]
struct Bytes<const N: usize>([u8; N]);

impl<const N: usize> Bytes<N> {
    fn new() -> Box<Self> {
        Box::new(Self([0; N]))
    }
    fn addr(&self) -> usize {
        self.0.as_ptr() as usize
    }
    fn put(&mut self, offset: usize, value: &[u8]) {
        self.0[offset..offset + value.len()].copy_from_slice(value);
    }
    fn u32(&mut self, offset: usize, value: u32) {
        self.put(offset, &value.to_le_bytes());
    }
    fn u64(&mut self, offset: usize, value: u64) {
        self.put(offset, &value.to_le_bytes());
    }
}

// The owned-memory construction follows tests/division_membership_acquisition.rs.
// Its helpers are private to that test module; this independent fixture uses a
// measured-empty combat vector so the retained scalar tests need no side graph.
struct Fixture {
    _root_slot: Box<Bytes<8>>,
    game_state: Box<Bytes<0x800>>,
    _uuid: Box<Bytes<40>>,
    primary: Box<Bytes<0x800>>,
    _stats: Box<Bytes<0x300>>,
    module: usize,
}

impl Fixture {
    fn new(raw: u8, owner: i32) -> Self {
        let mut root_slot = Bytes::new();
        let mut game_state = Bytes::new();
        let mut uuid = Bytes::new();
        let mut primary = Bytes::new();
        let mut stats = Bytes::new();
        let module = root_slot
            .addr()
            .checked_sub(profile::DOMAIN57_CGAMESTATE_SINGLETON_RVA)
            .unwrap();
        let body = DIVISION_WRAPPER_TO_BODY;
        root_slot.u64(0, game_state.addr() as u64);
        game_state.u32(GS_CURRENT_DATE_OFFSET, HOUR as u32);
        game_state.u32(
            profile::DOMAIN_CAMPAIGN_ID_GAME_UNIQUE_SEED_OFF,
            SEED as u32,
        );
        game_state.u64(
            profile::DOMAIN_CAMPAIGN_ID_GAME_UNIQUE_ID_OFF,
            uuid.addr() as u64,
        );
        game_state.u64(
            profile::DOMAIN_CAMPAIGN_ID_GAME_UNIQUE_ID_OFF + 0x10,
            UUID.len() as u64,
        );
        game_state.u64(
            profile::DOMAIN_CAMPAIGN_ID_GAME_UNIQUE_ID_OFF + 0x18,
            UUID.len() as u64,
        );
        uuid.put(0, &UUID);
        primary.u64(0, (module + DIVISION_WRAPPER_VTABLE_RVA) as u64);
        primary.u64(body, (module + DIVISION_VTABLE_RVA) as u64);
        primary.u32(body + DIVISION_OFF_ID_TYPE, DIVISION_TYPE);
        primary.u32(body + DIVISION_OFF_ID_ID, 7);
        primary.u32(body + DIVISION_OFF_LOCATION, 12);
        primary.u32(body + DIVISION_OFF_STRENGTH, 100_000);
        primary.u32(body + DIVISION_OFF_ORGANISATION, 200_000);
        primary.u32(body + DIVISION_OFF_OWNER_TAG_DB_ID, owner as u32);
        primary.put(body + DIVISION_OFF_RETAINED_VISUAL_VARIANT, &[raw]);
        primary.u32(0x1e0, (-7i32) as u32); // Independent training country operand.
        primary.u64(DIVISION_WRAPPER_OFF_COUNTER_STATS, stats.addr() as u64);
        stats.u64(0, (module + DIVISION_COUNTER_STATS_VTABLE_RVA) as u64);
        stats.u64(DIVISION_COUNTER_STATS_OFF_MAX_ORGANISATION, 300_000);
        stats.u64(0x288, 150_000);
        primary.u64(0x430, 42_000);
        primary.u64(body + 0x230, (-5i64) as u64);
        primary.put(body + 0x23c, &[255]);
        Self {
            _root_slot: root_slot,
            game_state,
            _uuid: uuid,
            primary,
            _stats: stats,
            module,
        }
    }

    fn entry(&self) -> DivisionBodyBridgeEntry {
        DivisionBodyBridgeEntry {
            div_id: 7,
            body_ptr: self.primary.addr() + DIVISION_WRAPPER_TO_BODY,
            owner_tag_db_id: 99, // Deliberately stale: it must never supply owner authority.
        }
    }
    fn frame(&self) -> MotionFrameInput {
        MotionFrameInput {
            game_state: self.game_state.addr(),
            raw_hour: HOUR,
        }
    }
    fn variant_address(&self) -> usize {
        self.entry().body_ptr + DIVISION_OFF_RETAINED_VISUAL_VARIANT
    }
    fn owner_address(&self) -> usize {
        self.entry().body_ptr + DIVISION_OFF_OWNER_TAG_DB_ID
    }
}

fn campaign() -> CampaignBinding<'static> {
    CampaignBinding {
        logical_stream_id: 1,
        game_unique_id: UUID,
        game_unique_seed: SEED,
        custody: None,
    }
}

#[derive(Default)]
struct Reads {
    visits: BTreeMap<(usize, usize), usize>,
    events: Vec<(usize, usize)>,
}

impl Reads {
    fn note(&mut self, at: usize, width: usize) -> usize {
        self.events.push((at, width));
        let visit = self.visits.entry((at, width)).or_default();
        *visit += 1;
        *visit
    }
    fn count(&self, at: usize, width: usize) -> usize {
        self.visits.get(&(at, width)).copied().unwrap_or(0)
    }
    fn assert_drained(&self) {
        assert!(!self.events.is_empty());
        for ((at, width), visits) in &self.visits {
            assert_eq!(
                visits % 2,
                0,
                "unclosed original address {at:#x}/{width}: {visits} visits"
            );
        }
    }
}

fn capture(
    fixture: &Fixture,
    budget: &Budget,
    mut after_read: impl FnMut(usize, usize, &mut [u8], bool) -> bool,
) -> (
    Result<Option<DivisionMovementSample>, DivisionSampleFailure>,
    Reads,
) {
    let mut reads = Reads::default();
    let result = sample_division_movement_with_training_reader(
        fixture.module,
        &fixture.entry(),
        fixture.frame(),
        Some(&campaign()),
        budget,
        &mut |at, out| {
            let visit = reads.note(at, out.len());
            let obtained = read_into(at, out);
            after_read(at, visit, out, obtained)
        },
        &mut || Some(ATTACHMENT),
    );
    (result, reads)
}

fn refusal(cause: Cause, field: Field, stage: Stage) -> Refusal {
    Refusal {
        cause,
        field,
        stage,
    }
}

fn assert_other_inputs(sample: &DivisionMovementSample) {
    assert_eq!(sample.raw_strength, 100_000);
    assert_eq!(sample.raw_organisation, 200_000);
    assert_eq!(sample.movement_scalars.numerator_raw, Some(-5));
    assert_eq!(sample.movement_scalars.retreat_raw, Some(255));
    assert_eq!(
        sample.training_scalars.country_operand,
        wire::TrainingObservation::Measured(-7)
    );
    assert_eq!(
        sample.first_combat_countries,
        wire::FirstCombatCountriesObservation::NoFirstCombat
    );
}

#[test]
fn actual_root_sampler_keeps_raw_zero_and_signed_byte_extremes_with_one_read_each_way() {
    let _counters = lock_rc_counters();
    for raw in [0, 127, 128, 255] {
        page_cache_new_frame();
        let fixture = Fixture::new(raw, 3);
        let budget = Budget::default();
        let (result, reads) = capture(&fixture, &budget, |_, _, _, obtained| obtained);
        let sample = result.unwrap().unwrap();
        assert_eq!(sample.retained_visual_variant, Variant::Measured(raw));
        assert_eq!(sample.owner_authority, Owner::Witnessed);
        assert_eq!(sample.owner_tag_db_id, Some(3));
        assert_other_inputs(&sample);
        assert_eq!(reads.count(fixture.variant_address(), 1), 2);
        assert_eq!(reads.count(fixture.owner_address(), 4), 2);
        assert!(reads
            .events
            .iter()
            .filter(|(at, _)| *at == fixture.variant_address())
            .all(|(_, width)| *width == 1));
        assert!(reads
            .events
            .iter()
            .filter(|(at, _)| *at == fixture.owner_address())
            .all(|(_, width)| *width == 4));
        assert_eq!(
            budget.verification_bytes(),
            reads.events.iter().map(|(_, width)| width).sum::<usize>()
        );
        reads.assert_drained();
    }
}

#[test]
fn actual_root_sampler_byte_open_and_close_loss_preserve_owner_and_other_inputs() {
    let _counters = lock_rc_counters();
    for (failed_visit, changed, cause, stage) in [
        (1, false, Cause::Unreadable, Stage::Open),
        (2, false, Cause::Unreadable, Stage::Close),
        (2, true, Cause::Changed, Stage::Close),
    ] {
        page_cache_new_frame();
        let fixture = Fixture::new(128, 3);
        let (result, reads) = capture(&fixture, &Budget::default(), |at, visit, out, obtained| {
            if at == fixture.variant_address() && visit == failed_visit {
                if changed {
                    out[0] ^= 1;
                    obtained
                } else {
                    false
                }
            } else {
                obtained
            }
        });
        let sample = result.unwrap().unwrap();
        assert_eq!(
            sample.retained_visual_variant,
            Variant::Unavailable(refusal(cause, Field::RetainedVisualVariant, stage))
        );
        assert_eq!(sample.owner_tag_db_id, Some(3));
        assert_eq!(sample.owner_authority, Owner::Witnessed);
        assert_other_inputs(&sample);
        assert_eq!(reads.count(fixture.variant_address(), 1), 2);
        assert_eq!(reads.count(fixture.owner_address(), 4), 2);
        reads.assert_drained();
    }
}

#[test]
fn actual_root_sampler_owner_open_and_close_loss_never_uses_cached_bridge_owner() {
    let _counters = lock_rc_counters();
    for (failed_visit, changed, cause, stage) in [
        (1, false, Cause::Unreadable, Stage::Open),
        (2, false, Cause::Unreadable, Stage::Close),
        (2, true, Cause::Changed, Stage::Close),
    ] {
        page_cache_new_frame();
        let fixture = Fixture::new(255, 3);
        let (result, reads) = capture(&fixture, &Budget::default(), |at, visit, out, obtained| {
            if at == fixture.owner_address() && visit == failed_visit {
                if changed {
                    out[..4].copy_from_slice(&99i32.to_le_bytes());
                    obtained
                } else {
                    false
                }
            } else {
                obtained
            }
        });
        let sample = result.unwrap().unwrap();
        assert_eq!(sample.owner_tag_db_id, None);
        assert_eq!(
            sample.owner_authority,
            Owner::Unavailable(refusal(cause, Field::DivisionOwner, stage))
        );
        assert_eq!(sample.retained_visual_variant, Variant::Measured(255));
        assert_other_inputs(&sample);
        assert_eq!(reads.count(fixture.owner_address(), 4), 2);
        reads.assert_drained();
    }
}

#[test]
fn actual_root_sampler_unsupported_signed_owner_preserves_projection_refusal() {
    let _counters = lock_rc_counters();
    for raw in [i32::MIN, -1, 0, MAX_DIVISION_OWNER_TAG_DB_ID as i32 + 1] {
        page_cache_new_frame();
        let fixture = Fixture::new(0, raw);
        let (result, reads) = capture(&fixture, &Budget::default(), |_, _, _, obtained| obtained);
        let sample = result.unwrap().unwrap();
        assert_eq!(sample.owner_tag_db_id, None);
        assert_eq!(
            sample.owner_authority,
            Owner::Unavailable(refusal(
                Cause::OwnerProjectionUnavailable,
                Field::DivisionOwner,
                Stage::Open
            ))
        );
        assert_eq!(sample.retained_visual_variant, Variant::Measured(0));
        assert_eq!(reads.count(fixture.owner_address(), 4), 2);
        reads.assert_drained();
    }
    for raw in [1, MAX_DIVISION_OWNER_TAG_DB_ID as i32] {
        page_cache_new_frame();
        let fixture = Fixture::new(0, raw);
        let (result, reads) = capture(&fixture, &Budget::default(), |_, _, _, obtained| obtained);
        let sample = result.unwrap().unwrap();
        assert_eq!(sample.owner_tag_db_id, Some(raw as u32));
        assert_eq!(sample.owner_authority, Owner::Witnessed);
        reads.assert_drained();
    }
}

#[test]
fn actual_root_sampler_mandatory_primary_close_failure_rejects_whole_row() {
    use division_combat_membership::{
        Cause as NativeCause, Field as NativeField, Stage as NativeStage,
    };
    let _counters = lock_rc_counters();
    for changed in [false, true] {
        page_cache_new_frame();
        let fixture = Fixture::new(128, 3);
        // Training opens/closes primary first; the motion adapter's original
        // primary opening and close are visits three and four respectively.
        let (result, reads) = capture(&fixture, &Budget::default(), |at, visit, out, obtained| {
            if at == fixture.primary.addr() && out.len() == 8 && visit == 4 {
                if changed {
                    out[0] ^= 8;
                    obtained
                } else {
                    false
                }
            } else {
                obtained
            }
        });
        let expected = if changed {
            NativeCause::Changed
        } else {
            NativeCause::Unreadable
        };
        assert!(
            matches!(result, Err(DivisionSampleFailure::FrameAuthorityUnavailable(reason))
            if reason.cause == expected && reason.field == NativeField::DivisionIdentity
                && reason.stage == NativeStage::Close)
        );
        assert_eq!(reads.count(fixture.primary.addr(), 8), 4);
        assert_eq!(reads.count(fixture.variant_address(), 1), 2);
        assert_eq!(reads.count(fixture.owner_address(), 4), 2);
        reads.assert_drained();
    }
}

#[test]
fn actual_root_sampler_campaign_mutation_during_leaf_closes_is_caught_by_final_authority_close() {
    use division_combat_membership::{
        Cause as NativeCause, Field as NativeField, Stage as NativeStage,
    };
    let _counters = lock_rc_counters();
    for mutate_at_variant in [true, false] {
        for mutate_uuid in [true, false] {
            page_cache_new_frame();
            let fixture = RefCell::new(Fixture::new(255, 3));
            let module = fixture.borrow().module;
            let entry = fixture.borrow().entry();
            let frame = fixture.borrow().frame();
            let variant = fixture.borrow().variant_address();
            let owner = fixture.borrow().owner_address();
            let mut reads = Reads::default();
            let attachment_visits = Cell::new(0);
            let variant_closed = Cell::new(false);
            let owner_closed = Cell::new(false);
            let mutated = Cell::new(false);
            let result = sample_division_movement_with_training_reader(
                module,
                &entry,
                frame,
                Some(&campaign()),
                &Budget::default(),
                &mut |at, out| {
                    let visit = reads.note(at, out.len());
                    let variant_close = at == variant && out.len() == 1 && visit == 2;
                    let owner_close = at == owner && out.len() == 4 && visit == 2;
                    if variant_close || owner_close {
                        assert_eq!(
                            attachment_visits.get(),
                            1,
                            "original frame authority must remain pending through both leaf closes"
                        );
                        if variant_close {
                            variant_closed.set(true);
                        }
                        if owner_close {
                            owner_closed.set(true);
                        }
                        if (mutate_at_variant && variant_close)
                            || (!mutate_at_variant && owner_close)
                        {
                            let mut fixture = fixture.borrow_mut();
                            if mutate_uuid {
                                fixture._uuid.put(0, b"b");
                            } else {
                                fixture.game_state.u32(
                                    profile::DOMAIN_CAMPAIGN_ID_GAME_UNIQUE_SEED_OFF,
                                    (SEED + 1) as u32,
                                );
                            }
                            mutated.set(true);
                        }
                    }
                    read_into(at, out)
                },
                &mut || {
                    attachment_visits.set(attachment_visits.get() + 1);
                    if attachment_visits.get() == 2 {
                        assert!(
                            variant_closed.get() && owner_closed.get(),
                            "attachment close must follow every funded leaf close"
                        );
                        assert!(mutated.get());
                    }
                    Some(ATTACHMENT)
                },
            );
            assert!(
                matches!(result, Err(DivisionSampleFailure::FrameAuthorityUnavailable(reason))
                if reason.cause == NativeCause::Changed && reason.field == NativeField::CampaignIdentity
                    && reason.stage == NativeStage::Close)
            );
            assert!(mutated.get());
            assert_eq!(attachment_visits.get(), 2);
            assert_eq!(reads.count(variant, 1), 2);
            assert_eq!(reads.count(owner, 4), 2);
            reads.assert_drained();
        }
    }
}

#[test]
fn actual_root_sampler_byte_refusal_and_exhausted_shared_budget_still_drain_funded_closes() {
    let _counters = lock_rc_counters();
    page_cache_new_frame();
    let fixture = Fixture::new(128, 3);
    let budget = Budget::default();
    let exhausted = Cell::new(false);
    let (result, reads) = capture(&fixture, &budget, |at, visit, out, obtained| {
        if at == fixture.variant_address() && out.len() == 1 && visit == 1 {
            assert!(budget.reserve_verification(usize::MAX).is_err());
            exhausted.set(true);
            false
        } else {
            obtained
        }
    });
    assert!(exhausted.get());
    let sample = result.unwrap().unwrap();
    assert_eq!(
        sample.retained_visual_variant,
        Variant::Unavailable(refusal(
            Cause::Unreadable,
            Field::RetainedVisualVariant,
            Stage::Open
        ))
    );
    assert_eq!(reads.count(fixture.variant_address(), 1), 2);
    assert!(budget.reserve_verification(1).is_err());
    reads.assert_drained();
}

#[test]
fn actual_root_sampler_byte_needs_both_open_and_close_bytes_before_first_read() {
    let _counters = lock_rc_counters();
    page_cache_new_frame();
    let fixture = Fixture::new(127, 3);
    let generous = Budget::default();
    let prefix = Cell::new(None);
    let (result, reads) = capture(&fixture, &generous, |at, visit, out, obtained| {
        if at == fixture.variant_address() && out.len() == 1 && visit == 1 {
            prefix.set(Some(generous.verification_bytes() - 2));
        }
        obtained
    });
    assert_eq!(
        result.unwrap().unwrap().retained_visual_variant,
        Variant::Measured(127)
    );
    reads.assert_drained();
    let prefix = prefix
        .get()
        .expect("actual production byte opening was reached");
    for remaining in [0, 1, 2] {
        page_cache_new_frame();
        let budget = Budget::test_limits(prefix + remaining, 16 * 1024 * 1024);
        let (result, reads) = capture(&fixture, &budget, |_, _, _, obtained| obtained);
        let sample = result.unwrap().unwrap();
        if remaining == 2 {
            assert_eq!(sample.retained_visual_variant, Variant::Measured(127));
            assert_eq!(reads.count(fixture.variant_address(), 1), 2);
        } else {
            assert_eq!(
                sample.retained_visual_variant,
                Variant::Unavailable(refusal(
                    Cause::VerificationBudgetExceeded,
                    Field::RetainedVisualVariant,
                    Stage::Admission
                ))
            );
            assert_eq!(reads.count(fixture.variant_address(), 1), 0);
        }
        reads.assert_drained();
    }
}

fn publication_row(variant: Variant, owner: Owner) -> DivisionMovementSample {
    let mut row = deleted_division_sample(7, DIVISION_TYPE);
    row.deleted = false;
    row.location = 12;
    row.raw_max_organisation = Some(300_000);
    row.retained_visual_variant = variant;
    row.owner_authority = owner;
    row.owner_tag_db_id = if owner == Owner::Witnessed {
        Some(3)
    } else {
        None
    };
    row
}

fn assert_payload(rows: &[DivisionMovementSample], flags: u32) {
    let payload = compact_divisions_data_payload(
        rows,
        0,
        flags,
        rows.len() as u32,
        rows.len() as u32,
        None,
        None,
        &Budget::default(),
    )
    .unwrap();
    let parsed = wire::DivisionRoutes::parse(&payload).unwrap();
    for (index, row) in rows.iter().enumerate() {
        assert_eq!(
            parsed.retained_visual_variant(index),
            (row.retained_visual_variant, row.owner_authority)
        );
    }
}

#[test]
fn explicit_funded_clone_and_payload_preserve_byte_and_each_typed_refusal() {
    let unavailable = refusal(Cause::Unreadable, Field::RetainedVisualVariant, Stage::Open);
    let owner_unavailable = refusal(Cause::Changed, Field::DivisionOwner, Stage::Close);
    for variant in [
        Variant::Measured(0),
        Variant::Measured(127),
        Variant::Measured(128),
        Variant::Measured(255),
        Variant::Unavailable(unavailable),
    ] {
        for owner in [Owner::Witnessed, Owner::Unavailable(owner_unavailable)] {
            let row = publication_row(variant, owner);
            let budget = Budget::test_limits(0, 0);
            let copied = try_clone_division_movement_sample(&row, &budget).unwrap();
            assert_eq!(copied, row);
            assert_eq!(copied.retained_visual_variant, variant);
            assert_eq!(copied.owner_authority, owner);
            assert_eq!(
                budget.retained_bytes(),
                0,
                "inline scalars do not invent heap ownership"
            );
            let json: serde_json::Value =
                serde_json::from_str(&format!("{{{}}}", division_sample_fields_json(&copied)))
                    .unwrap();
            assert_eq!(
                json["retained_visual_variant"],
                serde_json::from_str::<serde_json::Value>(&variant.json()).unwrap()
            );
            assert_eq!(
                json["owner_authority"],
                serde_json::from_str::<serde_json::Value>(&owner.json()).unwrap()
            );
            assert_payload(&[copied], 0);
        }
    }
}

#[test]
fn byte_only_and_refusal_cause_or_stage_changes_emit_real_deltas() {
    let rows = [
        publication_row(Variant::Measured(0), Owner::Witnessed),
        publication_row(Variant::Measured(128), Owner::Witnessed),
        publication_row(
            Variant::Unavailable(refusal(
                Cause::Unreadable,
                Field::RetainedVisualVariant,
                Stage::Open,
            )),
            Owner::Witnessed,
        ),
        publication_row(
            Variant::Unavailable(refusal(
                Cause::Unreadable,
                Field::RetainedVisualVariant,
                Stage::Close,
            )),
            Owner::Witnessed,
        ),
        publication_row(
            Variant::Unavailable(refusal(
                Cause::Changed,
                Field::RetainedVisualVariant,
                Stage::Close,
            )),
            Owner::Witnessed,
        ),
    ];
    let mut tracker = FrameJournalled::default();
    for (index, row) in rows.iter().enumerate() {
        if index != 0 {
            assert_eq!(
                division_movement_changed_fields(&rows[index - 1], row),
                Some(vec!["retained_visual_variant"])
            );
        }
        let (emitted, changed, flags) = compact_division_emitted_rows(
            vec![row.clone()],
            1,
            0,
            &mut tracker,
            &Budget::default(),
        )
        .unwrap();
        assert_eq!(emitted, [row.clone()]);
        assert_eq!(changed, 1);
        assert_eq!(flags & COMPACT_STATUS_DELTA_DATA != 0, index != 0);
        assert_payload(&emitted, flags);
        tracker.close_frame_transaction();
    }
    let (unchanged, changed, flags) = compact_division_emitted_rows(
        vec![rows.last().unwrap().clone()],
        1,
        0,
        &mut tracker,
        &Budget::default(),
    )
    .unwrap();
    assert!(unchanged.is_empty());
    assert_eq!(changed, 0);
    assert_ne!(flags & COMPACT_STATUS_DELTA_DATA, 0);
}

#[test]
fn owner_refusal_transition_and_deletion_emit_canonical_new_fields() {
    let variant = Variant::Measured(255);
    let mut tracker = FrameJournalled::default();
    let first = publication_row(
        variant,
        Owner::Unavailable(refusal(
            Cause::Unreadable,
            Field::DivisionOwner,
            Stage::Open,
        )),
    );
    let second = publication_row(
        variant,
        Owner::Unavailable(refusal(Cause::Changed, Field::DivisionOwner, Stage::Close)),
    );
    assert_eq!(
        division_movement_changed_fields(&first, &second),
        Some(vec!["owner_authority"])
    );
    for row in [first, second] {
        let (emitted, changed, flags) = compact_division_emitted_rows(
            vec![row.clone()],
            1,
            0,
            &mut tracker,
            &Budget::default(),
        )
        .unwrap();
        assert_eq!(emitted, [row]);
        assert_eq!(changed, 1);
        assert_payload(&emitted, flags);
        tracker.close_frame_transaction();
    }
    let (deleted, changed, flags) =
        compact_division_emitted_rows(Vec::new(), 0, 0, &mut tracker, &Budget::default()).unwrap();
    assert_eq!(deleted.len(), 1);
    assert!(deleted[0].deleted);
    assert_eq!(deleted[0].retained_visual_variant, Variant::default());
    assert_eq!(deleted[0].owner_authority, Owner::default());
    assert_eq!(deleted[0].owner_tag_db_id, None);
    assert_eq!(changed, 1);
    assert_ne!(flags & COMPACT_STATUS_DELTA_DATA, 0);
    assert_payload(&deleted, flags);
}

#[test]
fn witnessed_owner_value_change_emits_compact_delta_even_with_unchanged_authority() {
    let mut tracker = FrameJournalled::default();
    for (index, owner) in [3, 99].into_iter().enumerate() {
        let mut row = publication_row(Variant::Measured(128), Owner::Witnessed);
        row.owner_tag_db_id = Some(owner);
        let (emitted, changed, flags) = compact_division_emitted_rows(
            vec![row.clone()],
            1,
            0,
            &mut tracker,
            &Budget::default(),
        )
        .unwrap();
        assert_eq!(emitted, [row]);
        assert_eq!(changed, 1);
        assert_eq!(flags & COMPACT_STATUS_DELTA_DATA != 0, index != 0);
        assert_payload(&emitted, flags);
        tracker.close_frame_transaction();
    }
}
