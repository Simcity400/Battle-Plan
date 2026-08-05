use super::*;
use crate::body::{check_key, diff};
use crate::value::{escapes, quantize};
use crate::TickGrid;

fn grid() -> TickGrid {
    TickGrid::new(3).expect("3 is a legal grid")
}

fn collection() -> Collection {
    schema().collections[0].clone()
}

fn index_of(c: &Collection, id: u32) -> usize {
    c.value_fields()
        .position(|f| f.field_id == id)
        .expect("field is in the schema")
}

/// A raw 88-byte member row, built word by word the way the lane packs one
/// (`domain49_50_naval_combat_live.rs:427`-`:449`).
#[derive(Clone, Copy, Debug)]
struct Raw {
    combat_id: u32,
    generation: u64,
    flags: u32,
    side: u32,
    group_index: u32,
    member_index: u32,
    unit_type_tag: u32,
    ship_instance_id: u32,
    owner_tag_db_id: u32,
    designed_max_strength_raw: u32,
    battle_slot_index_raw: u32,
    state_bitmask: u32,
    engagement_weight_raw: i32,
    reserved: u32,
    task_force_id: u32,
    fleet_id: u32,
    strength_raw: u32,
    organisation_raw: u32,
    sunk_convoys: u32,
    experience_raw: u32,
    reserved_tail: u32,
}

impl Raw {
    fn bytes(&self) -> Vec<u8> {
        let mut row = Vec::with_capacity(ROW_LEN);
        row.extend_from_slice(&self.combat_id.to_le_bytes());
        row.extend_from_slice(&self.generation.to_le_bytes());
        row.extend_from_slice(&self.flags.to_le_bytes());
        row.extend_from_slice(&self.side.to_le_bytes());
        row.extend_from_slice(&self.group_index.to_le_bytes());
        row.extend_from_slice(&self.member_index.to_le_bytes());
        row.extend_from_slice(&self.unit_type_tag.to_le_bytes());
        row.extend_from_slice(&self.ship_instance_id.to_le_bytes());
        row.extend_from_slice(&self.owner_tag_db_id.to_le_bytes());
        row.extend_from_slice(&self.designed_max_strength_raw.to_le_bytes());
        row.extend_from_slice(&self.battle_slot_index_raw.to_le_bytes());
        row.extend_from_slice(&self.state_bitmask.to_le_bytes());
        row.extend_from_slice(&self.engagement_weight_raw.to_le_bytes());
        row.extend_from_slice(&self.reserved.to_le_bytes());
        row.extend_from_slice(&self.task_force_id.to_le_bytes());
        row.extend_from_slice(&self.fleet_id.to_le_bytes());
        row.extend_from_slice(&self.strength_raw.to_le_bytes());
        row.extend_from_slice(&self.organisation_raw.to_le_bytes());
        row.extend_from_slice(&self.sunk_convoys.to_le_bytes());
        row.extend_from_slice(&self.experience_raw.to_le_bytes());
        row.extend_from_slice(&self.reserved_tail.to_le_bytes());
        assert_eq!(row.len(), ROW_LEN);
        row
    }
}

/// The member-owned gates, which every row carries whatever it is.
const MEMBER_OWNED_FLAGS: u32 =
    OWNER_PRESENT | DESIGNED_MAX_STRENGTH_PRESENT | STATE_BITMASK_PRESENT | ENGAGEMENT_WEIGHT_PRESENT;

/// A live ship with a validated CShip join and every gate set — the shape 97.9%
/// of the capture's rows have.
fn ship() -> Raw {
    Raw {
        combat_id: 165_521,
        generation: 7,
        flags: SHIP_INSTANCE_PRESENT
            | CSHIP_JOIN_VALIDATED
            | MEMBER_OWNED_FLAGS
            | CSHIP_DERIVED_FLAGS,
        side: SIDE_ATTACKER,
        group_index: 0,
        member_index: 3,
        unit_type_tag: CSHIP_TYPE_TAG,
        ship_instance_id: 4_544,
        owner_tag_db_id: 368,
        designed_max_strength_raw: 32_000_000,
        battle_slot_index_raw: 11,
        state_bitmask: 0x02,
        engagement_weight_raw: 300_000,
        reserved: 0,
        task_force_id: 17_617,
        fleet_id: 17_732,
        strength_raw: 1_500_000,
        organisation_raw: 6_750_000,
        sunk_convoys: 121,
        experience_raw: 10_000_000,
        reserved_tail: 0,
    }
}

/// A convoy member: zero handle, so no CShip identity, no join and none of the
/// six join-derived fields (`attach_log.rs:40110`-`:40120`, lane `:2051`-`:2056`).
fn convoy() -> Raw {
    Raw {
        flags: IS_CONVOY | MEMBER_OWNED_FLAGS,
        unit_type_tag: 0,
        ship_instance_id: 0,
        task_force_id: 0,
        fleet_id: 0,
        strength_raw: 0,
        organisation_raw: 0,
        sunk_convoys: 0,
        experience_raw: 0,
        // Measured: every convoy row in the capture carries a present zero here.
        designed_max_strength_raw: 0,
        ..ship()
    }
}

/// A terminal-sunk ship whose CShip has left the live OOB, so the join was never
/// attempted (`attach_log.rs:40801`-`:40813`). The only shape on which a
/// non-convoy row may carry a clear join bit.
fn sunk_unjoined_ship() -> Raw {
    Raw {
        flags: SHIP_INSTANCE_PRESENT | MEMBER_OWNED_FLAGS,
        state_bitmask: FEX_STATE_SUNK,
        task_force_id: 0,
        fleet_id: 0,
        strength_raw: 0,
        organisation_raw: 0,
        sunk_convoys: 0,
        experience_raw: 0,
        ..ship()
    }
}

/// Wrap rows into a payload with the lane's header and its one empty string entry.
fn payload(status_flags: u32, rows: &[Vec<u8>]) -> Vec<u8> {
    payload_with(SCHEMA_VERSION, ROW_LEN as u16, status_flags, rows.len() as u32, 1, 0, rows)
}

#[allow(clippy::too_many_arguments)]
fn payload_with(
    schema_version: u16,
    row_len: u16,
    status_flags: u32,
    row_count: u32,
    string_count: u32,
    reserved: u32,
    rows: &[Vec<u8>],
) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&schema_version.to_le_bytes());
    out.extend_from_slice(&row_len.to_le_bytes());
    out.extend_from_slice(&0u64.to_le_bytes()); // elapsed_wall_ms
    out.extend_from_slice(&status_flags.to_le_bytes());
    out.extend_from_slice(&row_count.to_le_bytes());
    out.extend_from_slice(&string_count.to_le_bytes());
    out.extend_from_slice(&reserved.to_le_bytes());
    for row in rows {
        out.extend_from_slice(row);
    }
    out.extend_from_slice(&0u16.to_le_bytes()); // the one empty string entry
    out
}

/// A raw 72-byte D49 lifecycle row for the parent mirror.
#[derive(Clone, Copy, Debug)]
struct RawLifecycle {
    combat_id: u32,
    generation: u64,
    body_ptr: u64,
    flags: u32,
    location_id: u32,
    region_id: u32,
    attacker_member_count: u32,
    defender_member_count: u32,
    attacker_positioning_raw: i32,
    defender_positioning_raw: i32,
    attacker_anti_air_raw: i64,
    defender_anti_air_raw: i64,
    reserved_tail: u64,
}

impl RawLifecycle {
    fn bytes(&self) -> Vec<u8> {
        let mut row = Vec::with_capacity(LIFECYCLE_ROW_LEN);
        row.extend_from_slice(&self.combat_id.to_le_bytes());
        row.extend_from_slice(&self.generation.to_le_bytes());
        row.extend_from_slice(&self.body_ptr.to_le_bytes());
        row.extend_from_slice(&self.flags.to_le_bytes());
        row.extend_from_slice(&self.location_id.to_le_bytes());
        row.extend_from_slice(&self.region_id.to_le_bytes());
        row.extend_from_slice(&self.attacker_member_count.to_le_bytes());
        row.extend_from_slice(&self.defender_member_count.to_le_bytes());
        row.extend_from_slice(&self.attacker_positioning_raw.to_le_bytes());
        row.extend_from_slice(&self.defender_positioning_raw.to_le_bytes());
        row.extend_from_slice(&self.attacker_anti_air_raw.to_le_bytes());
        row.extend_from_slice(&self.defender_anti_air_raw.to_le_bytes());
        row.extend_from_slice(&self.reserved_tail.to_le_bytes());
        assert_eq!(row.len(), LIFECYCLE_ROW_LEN);
        row
    }
}

const D49_ALL_PRESENT: u32 = D49_LOCATION_PRESENT
    | D49_REGION_PRESENT
    | D49_ATTACKER_COUNT_PRESENT
    | D49_DEFENDER_COUNT_PRESENT
    | D49_ATTACKER_POSITIONING_PRESENT
    | D49_DEFENDER_POSITIONING_PRESENT
    | D49_ATTACKER_ANTI_AIR_PRESENT
    | D49_DEFENDER_ANTI_AIR_PRESENT;

fn lifecycle(combat_id: u32, generation: u64) -> RawLifecycle {
    RawLifecycle {
        combat_id,
        generation,
        body_ptr: 0x1234_5678_9abc,
        flags: D49_ALL_PRESENT,
        location_id: 4_242,
        region_id: 77,
        attacker_member_count: 3,
        defender_member_count: 2,
        attacker_positioning_raw: 50_000,
        defender_positioning_raw: 100_000,
        attacker_anti_air_raw: 123_456,
        defender_anti_air_raw: -1,
        reserved_tail: 0,
    }
}

fn lifecycle_delete(combat_id: u32, generation: u64) -> RawLifecycle {
    RawLifecycle {
        flags: D49_ROW_DELETED,
        location_id: 0,
        region_id: 0,
        attacker_member_count: 0,
        defender_member_count: 0,
        attacker_positioning_raw: 0,
        defender_positioning_raw: 0,
        attacker_anti_air_raw: 0,
        defender_anti_air_raw: 0,
        ..lifecycle(combat_id, generation)
    }
}

fn lifecycle_payload(status_flags: u32, rows: &[RawLifecycle]) -> Vec<u8> {
    let bytes: Vec<Vec<u8>> = rows.iter().map(|r| r.bytes()).collect();
    payload_with(
        SCHEMA_VERSION,
        LIFECYCLE_ROW_LEN as u16,
        status_flags,
        rows.len() as u32,
        1,
        0,
        &bytes,
    )
}

/// A state whose parent set already holds one combat, so member folds are legal.
fn state_with_parent(combat_id: u32, generation: u64) -> State {
    let mut state = State::new();
    let outcome = state
        .fold_lifecycle_payload(
            &lifecycle_payload(0, &[lifecycle(combat_id, generation)]),
            false,
            ParentPolicy::FoldFaithful,
        )
        .expect("well-formed lifecycle payload");
    assert_eq!(outcome, LifecycleOutcome::Applied);
    state
}

// --- Step 8.1: the schema validates ------------------------------------------

#[test]
fn the_schema_is_valid() {
    schema()
        .validate(grid().log2())
        .expect("d50 schema is well-formed");
}

// --- Step 8.5: shape and key --------------------------------------------------

#[test]
fn it_is_a_keyed_set_keyed_by_the_folds_own_tuple() {
    let c = collection();
    assert_eq!(c.shape, Shape::KeyedSet, "cumulative upserts, deletes remove");
    let keys = c.key_fields();
    assert_eq!(keys.len(), 4, "world_fold.rs:20153-:20158");
    let ids: Vec<u32> = keys.iter().map(|f| f.field_id).collect();
    assert_eq!(
        ids,
        vec![
            field::COMBAT_ID,
            field::GENERATION,
            field::SIDE,
            field::BATTLE_SLOT_INDEX
        ]
    );
    for k in &keys {
        assert_eq!(k.kind, Kind::Unsigned, "field {} kind", k.field_id);
        assert!(k.kind.legal_as_key());
        assert!(
            !k.optional && !k.nullable && !k.unknown_capable && k.derived.is_none(),
            "a key member is concrete"
        );
    }
    assert_eq!(c.fields.len(), 19, "4 key + 15 value fields");
    // The key a row produces is legal for those kinds, and matches `member_key`.
    let kinds: Vec<Kind> = keys.iter().map(|f| f.kind).collect();
    let k = key_from_row(&ship().bytes()).expect("keys");
    check_key(&kinds, &k).expect("key components are in range");
    assert_eq!(k, member_key(165_521, 7, SIDE_ATTACKER, 11));
}

#[test]
fn no_field_is_derived_and_the_content_id_sweep_is_recorded() {
    let c = collection();
    // §6 derived fields are clock-linear integers; nothing here changes
    // deterministically with the clock, so `frame_tick` never enters a value.
    assert!(c.fields.iter().all(|f| f.derived.is_none()));
    // The ContentId retrofit sweep (§13) needs "checked, and here they are",
    // not silence. d50 has exactly one, and it is a VALUE, not a key — the
    // lighter retrofit class, because no baseline byte layout depends on it.
    let content_ids: Vec<u32> = c
        .fields
        .iter()
        .filter(|f| f.kind == Kind::ContentId)
        .map(|f| f.field_id)
        .collect();
    assert_eq!(content_ids, vec![field::OWNER_TAG_DB_ID]);
    assert!(
        c.key_fields().iter().all(|f| f.kind != Kind::ContentId),
        "no ContentId key: the heavier retrofit class does not apply to d50"
    );
}

// --- Step 8.3: the four states, through the mapper on raw bytes ---------------

#[test]
fn the_schema_declares_exactly_the_states_the_mapper_can_emit() {
    let c = collection();
    // No field is nullable, and there is no case to write for one: §3's `null`
    // is "the relation genuinely points at nothing", and no authority assigns
    // that meaning to any clear gate in this row. Every clear gate is either a
    // concept that does not apply (a convoy's CShip fields -> absent) or a read
    // that was not answered (-> unknown). See the module docs.
    assert!(
        c.value_fields().all(|f| !f.nullable),
        "d50 declares no nullable field, so the states test writes no null case"
    );
    // There is also no legacy short row: the lane pins `row_len == 88` in the
    // header (`:1804`-`:1808`), so no field is unknown-because-the-row-is-short.
    let optional: Vec<u32> = c
        .value_fields()
        .filter(|f| f.optional)
        .map(|f| f.field_id)
        .collect();
    assert_eq!(
        optional,
        vec![
            field::SHIP_INSTANCE_ID,
            field::TASK_FORCE_ID,
            field::FLEET_ID,
            field::STRENGTH,
            field::ORGANISATION,
            field::EXPERIENCE,
            field::SUNK_CONVOYS
        ],
        "absent is exactly the CShip-identity and CShip-join fields"
    );
    let unknown_capable: Vec<u32> = c
        .value_fields()
        .filter(|f| f.unknown_capable)
        .map(|f| f.field_id)
        .collect();
    assert_eq!(
        unknown_capable,
        vec![
            field::OWNER_TAG_DB_ID,
            field::DESIGNED_MAX_STRENGTH_RAW,
            field::STATE,
            field::ENGAGEMENT_WEIGHT_RAW,
            field::TASK_FORCE_ID,
            field::FLEET_ID,
            field::STRENGTH,
            field::ORGANISATION,
            field::EXPERIENCE,
            field::SUNK_CONVOYS
        ],
        "unknown is exactly the member-owned reads and the CShip-join fields"
    );
    // `ship_instance_id` is optional and NOT unknown-capable: the survey's
    // reading has it governed by the join bit, but the lane requires it present
    // on every non-convoy row and forbids it on every convoy row.
    let ship_field = c
        .value_fields()
        .find(|f| f.field_id == field::SHIP_INSTANCE_ID)
        .expect("in schema");
    assert!(ship_field.optional && !ship_field.unknown_capable);
}

#[test]
fn a_full_ship_row_maps_every_field_to_a_value() {
    let c = collection();
    let r = record_from_row(&c, &ship().bytes(), 0).expect("maps");
    assert_eq!(r.values[index_of(&c, field::GROUP_INDEX)], Value::Unsigned(0));
    assert_eq!(r.values[index_of(&c, field::MEMBER_INDEX)], Value::Unsigned(3));
    assert_eq!(
        r.values[index_of(&c, field::UNIT_TYPE_TAG)],
        Value::Unsigned(u64::from(CSHIP_TYPE_TAG))
    );
    assert_eq!(
        r.values[index_of(&c, field::SHIP_INSTANCE_ID)],
        Value::Unsigned(4_544)
    );
    assert_eq!(
        r.values[index_of(&c, field::CSHIP_JOIN_VALIDATED)],
        Value::Bool(true)
    );
    assert_eq!(
        r.values[index_of(&c, field::OWNER_TAG_DB_ID)],
        Value::ContentId(368)
    );
    assert_eq!(
        r.values[index_of(&c, field::DESIGNED_MAX_STRENGTH_RAW)],
        Value::Unsigned(32_000_000),
        "carried raw: no fold divides it"
    );
    assert_eq!(r.values[index_of(&c, field::STATE)], Value::Enum(2));
    assert_eq!(
        r.values[index_of(&c, field::ENGAGEMENT_WEIGHT_RAW)],
        Value::Signed(300_000),
        "carried raw: no fold divides it"
    );
    assert_eq!(
        r.values[index_of(&c, field::TASK_FORCE_ID)],
        Value::Unsigned(17_617)
    );
    assert_eq!(r.values[index_of(&c, field::FLEET_ID)], Value::Unsigned(17_732));
    assert_eq!(
        r.values[index_of(&c, field::STRENGTH)],
        Value::Continuous(15.0)
    );
    assert_eq!(
        r.values[index_of(&c, field::ORGANISATION)],
        Value::Continuous(67.5)
    );
    assert_eq!(
        r.values[index_of(&c, field::EXPERIENCE)],
        Value::Continuous(100.0)
    );
    assert_eq!(
        r.values[index_of(&c, field::SUNK_CONVOYS)],
        Value::Unsigned(121)
    );
}

#[test]
fn a_convoy_row_makes_every_cship_concept_absent() {
    let c = collection();
    let r = record_from_row(&c, &convoy().bytes(), 0).expect("maps");
    for id in [
        field::SHIP_INSTANCE_ID,
        field::TASK_FORCE_ID,
        field::FLEET_ID,
        field::STRENGTH,
        field::ORGANISATION,
        field::EXPERIENCE,
        field::SUNK_CONVOYS,
    ] {
        assert_eq!(
            r.values[index_of(&c, id)],
            Value::Absent,
            "field {id} does not apply to a convoy member"
        );
    }
    // The member-owned reads still apply, and every one of them is a value.
    assert_eq!(
        r.values[index_of(&c, field::OWNER_TAG_DB_ID)],
        Value::ContentId(368)
    );
    assert_eq!(
        r.values[index_of(&c, field::DESIGNED_MAX_STRENGTH_RAW)],
        Value::Unsigned(0),
        "a present zero is a measured value, never an empty state"
    );
    assert_eq!(
        r.values[index_of(&c, field::CSHIP_JOIN_VALIDATED)],
        Value::Bool(false)
    );
    assert_eq!(
        r.values[index_of(&c, field::UNIT_TYPE_TAG)],
        Value::Unsigned(0),
        "the convoy witness rides in the tag"
    );
}

#[test]
fn an_unjoined_sunk_ship_makes_every_cship_join_field_unknown() {
    let c = collection();
    let r = record_from_row(&c, &sunk_unjoined_ship().bytes(), 0).expect("maps");
    for id in [
        field::TASK_FORCE_ID,
        field::FLEET_ID,
        field::STRENGTH,
        field::ORGANISATION,
        field::EXPERIENCE,
        field::SUNK_CONVOYS,
    ] {
        assert_eq!(
            r.values[index_of(&c, id)],
            Value::Unknown,
            "field {id}: the join was not answered, not answered-empty"
        );
    }
    // The ship's own identity is still carried — that is what distinguishes this
    // case from a convoy.
    assert_eq!(
        r.values[index_of(&c, field::SHIP_INSTANCE_ID)],
        Value::Unsigned(4_544)
    );
    assert_eq!(
        r.values[index_of(&c, field::CSHIP_JOIN_VALIDATED)],
        Value::Bool(false)
    );
    assert_eq!(r.values[index_of(&c, field::STATE)], Value::Enum(6));
}

#[test]
fn each_member_owned_gate_is_unknown_on_its_own_field_only() {
    let c = collection();
    // One gate clear at a time — the whole gate space, not one sampled case.
    let cases: [(u32, u32); 4] = [
        (OWNER_PRESENT, field::OWNER_TAG_DB_ID),
        (DESIGNED_MAX_STRENGTH_PRESENT, field::DESIGNED_MAX_STRENGTH_RAW),
        (STATE_BITMASK_PRESENT, field::STATE),
        (ENGAGEMENT_WEIGHT_PRESENT, field::ENGAGEMENT_WEIGHT_RAW),
    ];
    for (cleared, gated_id) in cases {
        let mut raw = ship();
        raw.flags &= !cleared;
        // A gated-off field must pack a zero (lane `:2097`-`:2146`).
        if cleared == OWNER_PRESENT {
            raw.owner_tag_db_id = 0;
        }
        if cleared == DESIGNED_MAX_STRENGTH_PRESENT {
            raw.designed_max_strength_raw = 0;
        }
        if cleared == STATE_BITMASK_PRESENT {
            raw.state_bitmask = 0;
        }
        if cleared == ENGAGEMENT_WEIGHT_PRESENT {
            raw.engagement_weight_raw = 0;
        }
        let r = record_from_row(&c, &raw.bytes(), 0)
            .unwrap_or_else(|e| panic!("maps with flags {:#x}: {e}", raw.flags));
        for f in c.value_fields() {
            assert_eq!(
                r.values[index_of(&c, f.field_id)] == Value::Unknown,
                f.field_id == gated_id,
                "field {} under flags {:#x}",
                f.field_id,
                raw.flags
            );
        }
    }
}

#[test]
fn a_real_zero_is_a_value_not_an_empty_state() {
    let c = collection();
    // Every gate set, every measurable value zero: the row a brand-new,
    // undamaged, inexperienced ship packs. §3 keeps zero out of the three
    // empty states.
    let raw = Raw {
        group_index: 0,
        member_index: 0,
        battle_slot_index_raw: 0,
        designed_max_strength_raw: 0,
        engagement_weight_raw: 0,
        strength_raw: 0,
        organisation_raw: 0,
        experience_raw: 0,
        sunk_convoys: 0,
        state_bitmask: 0,
        ..ship()
    };
    let r = record_from_row(&c, &raw.bytes(), 0).expect("maps");
    assert_eq!(r.values[index_of(&c, field::GROUP_INDEX)], Value::Unsigned(0));
    assert_eq!(
        r.values[index_of(&c, field::DESIGNED_MAX_STRENGTH_RAW)],
        Value::Unsigned(0)
    );
    assert_eq!(
        r.values[index_of(&c, field::ENGAGEMENT_WEIGHT_RAW)],
        Value::Signed(0)
    );
    assert_eq!(
        r.values[index_of(&c, field::STATE)],
        Value::Enum(0),
        "FEX_STATE_UNINITIALIZED is a measured code, not an empty state"
    );
    for id in [field::STRENGTH, field::ORGANISATION, field::EXPERIENCE] {
        assert_eq!(r.values[index_of(&c, id)], Value::Continuous(0.0), "field {id}");
    }
    assert_eq!(r.values[index_of(&c, field::SUNK_CONVOYS)], Value::Unsigned(0));
}

// --- Step 8.2: round trips ----------------------------------------------------

fn roundtrips(c: &Collection, row: &[u8]) {
    let record = record_from_row(c, row, 0).expect("maps");
    let mut bytes = Vec::new();
    record.encode(c, &mut bytes).expect("encodes");
    let (back, pos) = Record::decode(c, &bytes, 0).expect("decodes");
    assert_eq!(pos, bytes.len(), "consumes exactly its own bytes");
    assert_eq!(back, record, "decode reproduces the record");
    let mut reencoded = Vec::new();
    back.encode(c, &mut reencoded).expect("re-encodes");
    assert_eq!(reencoded, bytes, "decode/encode is byte-identical");
}

#[test]
fn every_row_shape_roundtrips_byte_for_byte() {
    let c = collection();
    roundtrips(&c, &ship().bytes());
    roundtrips(&c, &convoy().bytes());
    roundtrips(&c, &sunk_unjoined_ship().bytes());
    // And with each member-owned gate clear, so the unknown bitmap is exercised
    // alongside the presence bitmap.
    for cleared in [
        OWNER_PRESENT,
        DESIGNED_MAX_STRENGTH_PRESENT,
        STATE_BITMASK_PRESENT,
        ENGAGEMENT_WEIGHT_PRESENT,
    ] {
        let mut raw = ship();
        raw.flags &= !cleared;
        raw.owner_tag_db_id = if cleared == OWNER_PRESENT { 0 } else { raw.owner_tag_db_id };
        raw.designed_max_strength_raw = if cleared == DESIGNED_MAX_STRENGTH_PRESENT {
            0
        } else {
            raw.designed_max_strength_raw
        };
        raw.state_bitmask = if cleared == STATE_BITMASK_PRESENT { 0 } else { raw.state_bitmask };
        raw.engagement_weight_raw = if cleared == ENGAGEMENT_WEIGHT_PRESENT {
            0
        } else {
            raw.engagement_weight_raw
        };
        roundtrips(&c, &raw.bytes());
    }
}

// --- Step 8.6: every field's offset, and every header word --------------------

#[test]
fn every_field_reads_its_own_offset() {
    // Distinct values at every offset, so a transposed pair fails. `sunk_convoys`
    // at +76 and `experience_raw` at +80 are the pair the row order inverts
    // relative to the flag order — this is the test that catches it.
    let c = collection();
    let raw = Raw {
        combat_id: 0x0102_0304,
        generation: 0x1112_1314_1516_1718,
        flags: SHIP_INSTANCE_PRESENT
            | CSHIP_JOIN_VALIDATED
            | MEMBER_OWNED_FLAGS
            | CSHIP_DERIVED_FLAGS,
        side: SIDE_DEFENDER,
        group_index: 21,
        member_index: 22,
        unit_type_tag: CSHIP_TYPE_TAG,
        ship_instance_id: 23,
        owner_tag_db_id: 24,
        designed_max_strength_raw: 25,
        battle_slot_index_raw: 26,
        state_bitmask: 0x08,
        engagement_weight_raw: -27,
        reserved: 0,
        task_force_id: 28,
        fleet_id: 29,
        strength_raw: 30,
        organisation_raw: 31,
        sunk_convoys: 32,
        experience_raw: 33,
        reserved_tail: 0,
    };
    let bytes = raw.bytes();
    let parsed = parse_row(&bytes).expect("parses");
    assert_eq!(parsed.combat_id, 0x0102_0304);
    assert_eq!(parsed.generation, 0x1112_1314_1516_1718);
    assert_eq!(parsed.side_wire_id, SIDE_DEFENDER);
    assert_eq!(parsed.battle_slot_index_raw, 26);
    assert_eq!(
        key_from_row(&bytes).expect("keys"),
        member_key(0x0102_0304, 0x1112_1314_1516_1718, SIDE_DEFENDER, 26)
    );

    let r = record_from_row(&c, &bytes, 0).expect("maps");
    let expected: [(u32, Value); 15] = [
        (field::GROUP_INDEX, Value::Unsigned(21)),
        (field::MEMBER_INDEX, Value::Unsigned(22)),
        (field::UNIT_TYPE_TAG, Value::Unsigned(51)),
        (field::SHIP_INSTANCE_ID, Value::Unsigned(23)),
        (field::CSHIP_JOIN_VALIDATED, Value::Bool(true)),
        (field::OWNER_TAG_DB_ID, Value::ContentId(24)),
        (field::DESIGNED_MAX_STRENGTH_RAW, Value::Unsigned(25)),
        (field::STATE, Value::Enum(4)),
        (field::ENGAGEMENT_WEIGHT_RAW, Value::Signed(-27)),
        (field::TASK_FORCE_ID, Value::Unsigned(28)),
        (field::FLEET_ID, Value::Unsigned(29)),
        (field::STRENGTH, Value::Continuous(30.0 / FIXED_SCALE)),
        (field::ORGANISATION, Value::Continuous(31.0 / FIXED_SCALE)),
        (field::SUNK_CONVOYS, Value::Unsigned(32)),
        (field::EXPERIENCE, Value::Continuous(33.0 / FIXED_SCALE)),
    ];
    for (id, value) in expected {
        assert_eq!(r.values[index_of(&c, id)], value, "field {id}");
    }
}

#[test]
fn the_three_scaled_fields_use_the_100000_scale_and_the_rest_are_raw() {
    // The test that catches a 32768-vs-100000 mapping error: known fixed-point
    // words against their exact f64 readings, per field at its own offset.
    let c = collection();
    let raw = Raw {
        strength_raw: 100_000,
        organisation_raw: 250_000,
        experience_raw: 1,
        designed_max_strength_raw: 100_000,
        engagement_weight_raw: 100_000,
        sunk_convoys: 100_000,
        ..ship()
    };
    let r = record_from_row(&c, &raw.bytes(), 0).expect("maps");
    assert_eq!(r.values[index_of(&c, field::STRENGTH)], Value::Continuous(1.0));
    assert_eq!(
        r.values[index_of(&c, field::ORGANISATION)],
        Value::Continuous(2.5)
    );
    assert_eq!(
        r.values[index_of(&c, field::EXPERIENCE)],
        Value::Continuous(1.0 / 100_000.0)
    );
    // And the whole-scale claim: the other three integer channels are raw.
    assert_eq!(
        r.values[index_of(&c, field::DESIGNED_MAX_STRENGTH_RAW)],
        Value::Unsigned(100_000),
        "no fold divides designed_max_strength_raw"
    );
    assert_eq!(
        r.values[index_of(&c, field::ENGAGEMENT_WEIGHT_RAW)],
        Value::Signed(100_000),
        "no fold divides engagement_weight_raw"
    );
    assert_eq!(
        r.values[index_of(&c, field::SUNK_CONVOYS)],
        Value::Unsigned(100_000),
        "sunk_convoys is a count with no denominator"
    );
}

#[test]
fn fixed_five_is_lossless_over_the_producers_whole_range() {
    // `Fixed(5)` is claimed as the source's own grid, not a precision budget.
    // The claim is that `quantize(raw / 1e5, Fixed(5)) == raw` for every value
    // the producer can emit — 0 ..= MAX_FIXED_POINT_RAW (lane `:2181`).
    let quant = Quant::Fixed(SCALE_DIGITS);
    let check = |raw: u32| {
        let v = f64::from(raw) / FIXED_SCALE;
        assert!(!escapes(v, quant), "raw {raw} escapes");
        assert_eq!(quantize(v, quant), Some(i64::from(raw)), "raw {raw}");
    };
    for raw in 0..20_000u32 {
        check(raw);
    }
    for raw in [
        1u32,
        99_999,
        100_000,
        100_001,
        999_999,
        1_000_000,
        123_456_789,
        MAX_FIXED_POINT_RAW - 1,
        MAX_FIXED_POINT_RAW,
    ] {
        check(raw);
    }
    // A dense sweep near the top of the range, where an f64 grid error would
    // show first.
    for raw in (MAX_FIXED_POINT_RAW - 5_000)..=MAX_FIXED_POINT_RAW {
        check(raw);
    }
}

// --- Step 8.4: quantization saves bytes ---------------------------------------

#[test]
fn quantization_saves_bytes_against_an_all_exact_variant() {
    let quantized = collection();
    let mut exact = quantized.clone();
    for f in &mut exact.fields {
        if f.kind == Kind::Continuous {
            f.quant = Quant::Exact;
        }
    }
    assert_eq!(
        exact.fields.iter().filter(|f| f.kind == Kind::Continuous).count(),
        3
    );
    let raw = ship().bytes();
    let mut q_bytes = Vec::new();
    record_from_row(&quantized, &raw, 0)
        .expect("maps")
        .encode(&quantized, &mut q_bytes)
        .expect("encodes");
    let mut e_bytes = Vec::new();
    record_from_row(&exact, &raw, 0)
        .expect("maps")
        .encode(&exact, &mut e_bytes)
        .expect("encodes");
    assert!(
        q_bytes.len() < e_bytes.len(),
        "fixed(5) {} bytes must beat exact {} bytes",
        q_bytes.len(),
        e_bytes.len()
    );
}

// --- The two witnesses --------------------------------------------------------

#[test]
fn is_convoy_is_a_total_function_of_the_unit_type_tag() {
    // The identity that lets `IS_CONVOY` stay off the wire. Swept over the whole
    // (is_convoy, tag) space the row can express, at the four tags that matter.
    for convoy_bit in [false, true] {
        for tag in [0u32, 1, CSHIP_TYPE_TAG, u32::MAX] {
            let mut raw = if convoy_bit { convoy() } else { ship() };
            raw.unit_type_tag = tag;
            let parsed = parse_row(&raw.bytes());
            let legal = (convoy_bit && tag == 0) || (!convoy_bit && tag == CSHIP_TYPE_TAG);
            assert_eq!(
                parsed.is_ok(),
                legal,
                "convoy={convoy_bit} tag={tag} must be {}",
                if legal { "accepted" } else { "rejected" }
            );
            if let Ok(row) = parsed {
                assert_eq!(
                    row.is_convoy,
                    row.unit_type_tag == 0,
                    "is_convoy is reconstructible from the tag alone"
                );
            }
        }
    }
}

#[test]
fn the_join_bit_is_carried_because_it_is_not_derivable() {
    // The lane accepts a validated join with every derived flag clear — it only
    // forbids the converse — so the join bit is not a function of the other
    // carried fields and must ride on the wire.
    let c = collection();
    let raw = Raw {
        flags: SHIP_INSTANCE_PRESENT | CSHIP_JOIN_VALIDATED | MEMBER_OWNED_FLAGS,
        task_force_id: 0,
        fleet_id: 0,
        strength_raw: 0,
        organisation_raw: 0,
        sunk_convoys: 0,
        experience_raw: 0,
        ..ship()
    };
    let joined = record_from_row(&c, &raw.bytes(), 0).expect("the fold accepts this row");
    assert_eq!(
        joined.values[index_of(&c, field::CSHIP_JOIN_VALIDATED)],
        Value::Bool(true)
    );
    // Same derived flags, join clear (the terminal-sunk shape): the join bit is
    // the only field that differs, which is exactly why it is carried.
    let unjoined = record_from_row(&c, &sunk_unjoined_ship().bytes(), 0).expect("maps");
    assert_eq!(
        unjoined.values[index_of(&c, field::CSHIP_JOIN_VALIDATED)],
        Value::Bool(false)
    );
    // Both map the six join fields to Unknown, so nothing else distinguishes them.
    for id in [field::TASK_FORCE_ID, field::STRENGTH] {
        assert_eq!(joined.values[index_of(&c, id)], Value::Unknown);
        assert_eq!(unjoined.values[index_of(&c, id)], Value::Unknown);
    }
}

#[test]
fn the_state_enum_is_a_bijection_over_the_lanes_seven_words() {
    for (code, raw) in FEX_STATES.iter().copied().enumerate() {
        assert_eq!(state_code(raw), Some(code as u32));
        assert_eq!(state_raw(code as u32), Some(raw));
    }
    assert_eq!(state_raw(FEX_STATES.len() as u32), None);
    // Every word outside the seven is rejected, not coerced to a code.
    for raw in 0..256u32 {
        assert_eq!(
            state_code(raw).is_some(),
            FEX_STATES.contains(&raw),
            "raw {raw:#x}"
        );
    }
    // And the schema's cardinality matches, so no legal code is rejected by the
    // encoder and no illegal one is admitted.
    let c = collection();
    let f = c
        .value_fields()
        .find(|f| f.field_id == field::STATE)
        .expect("in schema");
    assert_eq!(f.enum_card, Some(FEX_STATES.len() as u32));
    for (code, raw) in FEX_STATES.iter().copied().enumerate() {
        let mut r = ship();
        r.state_bitmask = raw;
        // Only the terminal-sunk state may carry a clear join bit, so keep the
        // join set for every code.
        let mapped = record_from_row(&c, &r.bytes(), 0).expect("maps");
        assert_eq!(mapped.values[index_of(&c, field::STATE)], Value::Enum(code as u32));
        roundtrips(&c, &r.bytes());
    }
}

// --- Mirroring the fold: what the mapper refuses ------------------------------

#[test]
fn the_mapper_rejects_every_row_the_lane_rejects() {
    let c = collection();
    // Wrong length, both directions, including the empty slice.
    for len in [0usize, 1, ROW_LEN - 1, ROW_LEN + 1, 72] {
        assert!(
            record_from_row(&c, &vec![0u8; len], 0).is_err(),
            "length {len} must be rejected, not read"
        );
        assert!(key_from_row(&vec![0u8; len]).is_err());
    }
    // Every flag bit outside KNOWN_FLAGS (lane `:1972`).
    // Bit 15 is ASSIGNED (`COMPACT_STATUS_BASELINE_CLEAR`, the stream's deliberate
    // resync primitive), so the UNASSIGNED run starts at 16.
    for bit in 16..32u32 {
        let mut raw = ship();
        raw.flags |= 1 << bit;
        assert!(parse_row(&raw.bytes()).is_err(), "flag bit {bit}");
    }
    // A zero or out-of-range combat_id (lane `:1979`).
    for id in [0u32, MAX_COMBAT_ID + 1, u32::MAX] {
        let mut raw = ship();
        raw.combat_id = id;
        assert!(parse_row(&raw.bytes()).is_err(), "combat_id {id}");
    }
    // Every side word other than 1 and 2 (lane `:1985`).
    for side in [0u32, 3, u32::MAX] {
        let mut raw = ship();
        raw.side = side;
        assert!(parse_row(&raw.bytes()).is_err(), "side {side}");
    }
    // The two reserved words (lane `:1991`, `:1997`).
    for (label, mutate) in [
        ("reserved", (|r: &mut Raw| r.reserved = 1) as fn(&mut Raw)),
        ("reserved_tail", |r: &mut Raw| r.reserved_tail = 1),
    ] {
        let mut raw = ship();
        mutate(&mut raw);
        assert!(parse_row(&raw.bytes()).is_err(), "{label}");
    }
    // Index bounds, each at max and max + 1 so the bound is not off by one.
    for (label, max, mutate) in [
        (
            "group_index",
            MAX_GROUP_INDEX,
            (|r: &mut Raw, v: u32| r.group_index = v) as fn(&mut Raw, u32),
        ),
        ("member_index", MAX_MEMBER_INDEX, |r: &mut Raw, v: u32| {
            r.member_index = v
        }),
        (
            "battle_slot_index_raw",
            MAX_BATTLE_SLOT_INDEX,
            |r: &mut Raw, v: u32| r.battle_slot_index_raw = v,
        ),
    ] {
        let mut ok = ship();
        mutate(&mut ok, max);
        assert!(parse_row(&ok.bytes()).is_ok(), "{label} at {max}");
        let mut bad = ship();
        mutate(&mut bad, max + 1);
        assert!(parse_row(&bad.bytes()).is_err(), "{label} at {}", max + 1);
    }
    // Present-value bounds on every bounded gated field.
    for (label, max, mutate) in [
        (
            "ship_instance_id",
            MAX_SHIP_INSTANCE_ID,
            (|r: &mut Raw, v: u32| r.ship_instance_id = v) as fn(&mut Raw, u32),
        ),
        ("owner_tag_db_id", MAX_OWNER_TAG_DB_ID, |r: &mut Raw, v| {
            r.owner_tag_db_id = v
        }),
        (
            "designed_max_strength_raw",
            MAX_DESIGNED_STRENGTH_RAW,
            |r: &mut Raw, v| r.designed_max_strength_raw = v,
        ),
        ("task_force_id", MAX_OOB_ID, |r: &mut Raw, v| {
            r.task_force_id = v
        }),
        ("fleet_id", MAX_OOB_ID, |r: &mut Raw, v| r.fleet_id = v),
        ("strength_raw", MAX_FIXED_POINT_RAW, |r: &mut Raw, v| {
            r.strength_raw = v
        }),
        ("organisation_raw", MAX_FIXED_POINT_RAW, |r: &mut Raw, v| {
            r.organisation_raw = v
        }),
        ("experience_raw", MAX_FIXED_POINT_RAW, |r: &mut Raw, v| {
            r.experience_raw = v
        }),
        ("sunk_convoys", MAX_SUNK_CONVOYS, |r: &mut Raw, v| {
            r.sunk_convoys = v
        }),
    ] {
        let mut ok = ship();
        mutate(&mut ok, max);
        assert!(parse_row(&ok.bytes()).is_ok(), "{label} at {max}");
        let mut bad = ship();
        mutate(&mut bad, max + 1);
        assert!(parse_row(&bad.bytes()).is_err(), "{label} at {}", max + 1);
    }
    // Zero-rejected present ids (lane `:2035`, `:2149`, `:2160`).
    for (label, mutate) in [
        (
            "ship_instance_id",
            (|r: &mut Raw| r.ship_instance_id = 0) as fn(&mut Raw),
        ),
        ("task_force_id", |r: &mut Raw| r.task_force_id = 0),
        ("fleet_id", |r: &mut Raw| r.fleet_id = 0),
    ] {
        let mut raw = ship();
        mutate(&mut raw);
        assert!(parse_row(&raw.bytes()).is_err(), "{label} present and zero");
    }
    // A gated-off field carrying a non-zero word, one field at a time.
    let gated: [(u32, fn(&mut Raw)); 11] = [
        (SHIP_INSTANCE_PRESENT, |r| r.ship_instance_id = 1),
        (OWNER_PRESENT, |r| r.owner_tag_db_id = 1),
        (DESIGNED_MAX_STRENGTH_PRESENT, |r| {
            r.designed_max_strength_raw = 1
        }),
        (STATE_BITMASK_PRESENT, |r| r.state_bitmask = 1),
        (ENGAGEMENT_WEIGHT_PRESENT, |r| r.engagement_weight_raw = 1),
        (TASK_FORCE_PRESENT, |r| r.task_force_id = 1),
        (FLEET_PRESENT, |r| r.fleet_id = 1),
        (STRENGTH_PRESENT, |r| r.strength_raw = 1),
        (ORGANISATION_PRESENT, |r| r.organisation_raw = 1),
        (EXPERIENCE_PRESENT, |r| r.experience_raw = 1),
        (SUNK_CONVOYS_PRESENT, |r| r.sunk_convoys = 1),
    ];
    for (flag, mutate) in gated {
        // Start from a shape in which clearing the flag is otherwise legal: a
        // sunk unjoined ship has every join gate clear already.
        let mut raw = sunk_unjoined_ship();
        raw.flags &= !flag;
        if flag == SHIP_INSTANCE_PRESENT {
            // A non-convoy row must carry a ship id, so use a convoy to test the
            // gated-off-but-non-zero rule for this one field.
            raw = convoy();
        }
        mutate(&mut raw);
        assert!(
            parse_row(&raw.bytes()).is_err(),
            "flag {flag:#x} clear with a non-zero word"
        );
    }
    // A state word outside the seven (lane `:2135`).
    for bad in [3u32, 0x21, 0x40, u32::MAX] {
        let mut raw = ship();
        raw.state_bitmask = bad;
        assert!(parse_row(&raw.bytes()).is_err(), "state {bad:#x}");
    }
    // The CShip-join structural rules (lane `:2045`-`:2096`).
    let mut join_without_ship = convoy();
    join_without_ship.flags |= CSHIP_JOIN_VALIDATED;
    assert!(parse_row(&join_without_ship.bytes()).is_err());
    let mut convoy_with_ship = convoy();
    convoy_with_ship.flags |= SHIP_INSTANCE_PRESENT;
    convoy_with_ship.ship_instance_id = 5;
    assert!(parse_row(&convoy_with_ship.bytes()).is_err());
    let mut unjoined_not_sunk = sunk_unjoined_ship();
    unjoined_not_sunk.state_bitmask = 0x02;
    assert!(
        parse_row(&unjoined_not_sunk.bytes()).is_err(),
        "only a terminal-sunk ship may outlive its join"
    );
    for derived in [
        TASK_FORCE_PRESENT,
        FLEET_PRESENT,
        STRENGTH_PRESENT,
        ORGANISATION_PRESENT,
        EXPERIENCE_PRESENT,
        SUNK_CONVOYS_PRESENT,
    ] {
        let mut unjoined = sunk_unjoined_ship();
        unjoined.flags |= derived;
        assert!(
            parse_row(&unjoined.bytes()).is_err(),
            "derived flag {derived:#x} without a join"
        );
        let mut c = convoy();
        c.flags |= derived;
        assert!(
            parse_row(&c.bytes()).is_err(),
            "derived flag {derived:#x} on a convoy"
        );
    }
}

#[test]
fn a_delete_row_is_a_tombstone_not_a_record() {
    let c = collection();
    let mut raw = ship();
    raw.flags |= ROW_DELETED;
    // The lane validates a delete row exactly like a live one, so it parses...
    let parsed = parse_row(&raw.bytes()).expect("a delete row is well formed");
    assert!(parsed.deleted);
    // ...but it has no record: quietly returning one would turn a delete into an
    // upsert.
    assert!(record_from_row(&c, &raw.bytes(), 0).is_err());
}

#[test]
fn the_mapper_refuses_a_collection_it_cannot_fill() {
    // A caller handing over another domain's collection must not get a record
    // full of placeholders under this domain's field ids.
    let mut wrong_id = collection();
    wrong_id.collection_id = 99;
    assert!(record_from_row(&wrong_id, &ship().bytes(), 0).is_err());
    let mut extra_field = collection();
    extra_field.fields.push(Field::new(999, Kind::Unsigned));
    assert!(record_from_row(&extra_field, &ship().bytes(), 0).is_err());
}

// --- split_payload and the header --------------------------------------------

#[test]
fn split_payload_enforces_the_header_word_by_word() {
    let rows = vec![ship().bytes()];
    let good = payload(0x400, &rows);
    let (header, body) = split_payload(&good).expect("well-formed payload");
    assert_eq!(header.schema_version, SCHEMA_VERSION);
    assert_eq!(header.row_len as usize, ROW_LEN);
    assert_eq!(header.elapsed_wall_ms, 0);
    assert_eq!(header.status_flags, 0x400);
    assert_eq!(header.row_count, 1);
    assert_eq!(header.string_count, 1);
    assert_eq!(header.reserved, 0);
    assert_eq!(body, &rows[0][..]);

    // Short of the header, at every length.
    for len in 0..HEADER_LEN {
        assert!(split_payload(&good[..len]).is_err(), "len {len}");
    }
    // Each header word wrong, one at a time.
    assert!(split_payload(&payload_with(1, ROW_LEN as u16, 0, 1, 1, 0, &rows)).is_err());
    assert!(split_payload(&payload_with(3, ROW_LEN as u16, 0, 1, 1, 0, &rows)).is_err());
    assert!(split_payload(&payload_with(SCHEMA_VERSION, 72, 0, 1, 1, 0, &rows)).is_err());
    assert!(split_payload(&payload_with(SCHEMA_VERSION, 87, 0, 1, 1, 0, &rows)).is_err());
    assert!(split_payload(&payload_with(SCHEMA_VERSION, ROW_LEN as u16, 0, 1, 0, 0, &rows)).is_err());
    assert!(split_payload(&payload_with(SCHEMA_VERSION, ROW_LEN as u16, 0, 1, 2, 0, &rows)).is_err());
    assert!(split_payload(&payload_with(SCHEMA_VERSION, ROW_LEN as u16, 0, 1, 1, 1, &rows)).is_err());
    // A row_count that does not match the body, both directions.
    assert!(split_payload(&payload_with(SCHEMA_VERSION, ROW_LEN as u16, 0, 0, 1, 0, &rows)).is_err());
    assert!(split_payload(&payload_with(SCHEMA_VERSION, ROW_LEN as u16, 0, 2, 1, 0, &rows)).is_err());
    // Above the lane's row-count limit, and a count whose byte length overflows
    // `usize` rather than wrapping into a plausible small length.
    assert!(split_payload(&payload_with(
        SCHEMA_VERSION,
        ROW_LEN as u16,
        0,
        MAX_ROW_COUNT + 1,
        1,
        0,
        &rows
    ))
    .is_err());
    assert!(split_payload(&payload_with(
        SCHEMA_VERSION,
        ROW_LEN as u16,
        0,
        u32::MAX,
        1,
        0,
        &rows
    ))
    .is_err());
    // Trailing bytes after the string table (lane `:1862`).
    let mut trailing = good.clone();
    trailing.push(0);
    assert!(split_payload(&trailing).is_err());
    // A truncated string table.
    assert!(split_payload(&good[..good.len() - 1]).is_err());
    // A non-empty entry at index 0 (lane `:1852`).
    let mut nonempty = good.clone();
    let tail = nonempty.len() - 2;
    nonempty[tail..].copy_from_slice(&1u16.to_le_bytes());
    nonempty.push(b'x');
    assert!(split_payload(&nonempty).is_err());
    // A bad row fails the whole payload, exactly as the lane's decode does.
    let mut bad_row = ship();
    bad_row.reserved = 1;
    assert!(split_payload(&payload(0, &[bad_row.bytes()])).is_err());
    // An empty payload is legal: a delta hour with no changed member.
    let empty = payload(0x400, &[]);
    let (h, b) = split_payload(&empty).expect("an empty delta is well formed");
    assert_eq!(h.row_count, 0);
    assert!(b.is_empty());
    // Arbitrary bytes never panic.
    for len in 0..(HEADER_LEN + ROW_LEN + 8) {
        let _ = split_payload(&vec![0xA5u8; len]);
        let _ = split_lifecycle_payload(&vec![0x5Au8; len]);
    }
}

// --- The frame-level fold -----------------------------------------------------

#[test]
fn the_fold_applies_upserts_and_deletes() {
    let c = collection();
    let mut state = state_with_parent(165_521, 7);
    assert_eq!(
        state.fold_payload(&c, &payload(0x400, &[ship().bytes()])),
        Ok(FoldOutcome::Applied)
    );
    assert_eq!(state.members.len(), 1);
    assert!(state.retired().is_empty());

    // A delete removes the key AND retires it, permanently.
    let mut gone = ship();
    gone.flags |= ROW_DELETED;
    assert_eq!(
        state.fold_payload(&c, &payload(0x400, &[gone.bytes()])),
        Ok(FoldOutcome::Applied)
    );
    assert!(state.members.is_empty());
    assert_eq!(state.retired().len(), 1);
}

#[test]
fn an_orphan_member_refuses_the_hour_without_mutating_anything() {
    let c = collection();
    let mut state = state_with_parent(165_521, 7);
    state
        .fold_payload(&c, &payload(0x400, &[ship().bytes()]))
        .expect("folds");
    let before = state.clone();

    let mut orphan = ship();
    orphan.combat_id = 999_999;
    let mut victim = ship();
    victim.battle_slot_index_raw = 12;
    victim.strength_raw = 1;
    // The refusal is decided over the WHOLE payload before any mutation, so the
    // legitimate row beside the orphan must not land either.
    assert_eq!(
        state.fold_payload(&c, &payload(0x400, &[victim.bytes(), orphan.bytes()])),
        Ok(FoldOutcome::OrphanMemberReference)
    );
    assert_eq!(state, before, "the accumulator is retained untouched");

    // A DELETE row for an unknown combat is exempt — an idempotent no-op — and
    // still retires its key (`world_fold.rs:20202`-`:20213`).
    let mut orphan_delete = orphan;
    orphan_delete.flags |= ROW_DELETED;
    assert_eq!(
        state.fold_payload(&c, &payload(0x400, &[orphan_delete.bytes()])),
        Ok(FoldOutcome::Applied)
    );
    assert!(state.retired().contains(&(999_999, 7, SIDE_ATTACKER, 11)));
    assert_eq!(state.members, before.members, "no member moved");
}

#[test]
fn a_duplicate_identity_refuses_the_hour() {
    let c = collection();
    let mut state = state_with_parent(165_521, 7);
    let before = state.clone();
    let mut second = ship();
    second.member_index = 4;
    assert_eq!(
        state.fold_payload(&c, &payload(0x400, &[ship().bytes(), second.bytes()])),
        Ok(FoldOutcome::DuplicateMemberIdentity)
    );
    assert_eq!(state, before);
    // The check runs over deleted rows too (`world_fold.rs:20155`-`:20161`).
    let mut deleted = ship();
    deleted.flags |= ROW_DELETED;
    assert_eq!(
        state.fold_payload(&c, &payload(0x400, &[ship().bytes(), deleted.bytes()])),
        Ok(FoldOutcome::DuplicateMemberIdentity)
    );
    assert_eq!(state, before);
    // A different slot is a different identity and folds.
    let mut other_slot = ship();
    other_slot.battle_slot_index_raw = 12;
    assert_eq!(
        state.fold_payload(&c, &payload(0x400, &[ship().bytes(), other_slot.bytes()])),
        Ok(FoldOutcome::Applied)
    );
    assert_eq!(state.members.len(), 2);
    // ... and so is a different side at the same slot.
    let mut other_side = ship();
    other_side.side = SIDE_DEFENDER;
    assert_eq!(
        state.fold_payload(&c, &payload(0x400, &[other_side.bytes()])),
        Ok(FoldOutcome::Applied)
    );
    assert_eq!(state.members.len(), 3);
}

#[test]
fn a_retired_key_can_never_be_resurrected_and_the_set_outlives_the_hour() {
    let c = collection();
    let mut state = state_with_parent(165_521, 7);
    state
        .fold_payload(&c, &payload(0x400, &[ship().bytes()]))
        .expect("folds");
    let mut gone = ship();
    gone.flags |= ROW_DELETED;
    state
        .fold_payload(&c, &payload(0x400, &[gone.bytes()]))
        .expect("folds");

    // Many hours later — the set is never cleared anywhere in world_fold.rs.
    for _ in 0..10 {
        state
            .fold_payload(&c, &payload(0x400, &[]))
            .expect("an empty delta folds");
    }
    let before = state.clone();
    assert_eq!(
        state.fold_payload(&c, &payload(0x400, &[ship().bytes()])),
        Ok(FoldOutcome::RetiredMemberResurrection)
    );
    assert_eq!(state, before, "the accumulator is retained untouched");
    // A DELETE for a retired key is not a resurrection (the refusal reads
    // `!row.deleted`, `world_fold.rs:20178`).
    assert_eq!(
        state.fold_payload(&c, &payload(0x400, &[gone.bytes()])),
        Ok(FoldOutcome::Applied)
    );
}

#[test]
fn the_refusals_are_checked_in_the_folds_own_order() {
    // A payload that trips more than one refusal reports the first one the fold
    // would: orphan, then duplicate, then resurrection
    // (`world_fold.rs:20146`, `:20153`, `:20167`).
    let c = collection();
    let mut state = state_with_parent(165_521, 7);
    let mut orphan = ship();
    orphan.combat_id = 999_999;
    let mut orphan_twin = orphan;
    orphan_twin.member_index = 9;
    assert_eq!(
        state.fold_payload(&c, &payload(0x400, &[orphan.bytes(), orphan_twin.bytes()])),
        Ok(FoldOutcome::OrphanMemberReference),
        "orphan is checked before duplicate identity"
    );
    // Retire a key, then hand over a payload that both duplicates it and
    // resurrects it.
    let mut gone = ship();
    gone.flags |= ROW_DELETED;
    state
        .fold_payload(&c, &payload(0x400, &[ship().bytes()]))
        .expect("folds");
    state
        .fold_payload(&c, &payload(0x400, &[gone.bytes()]))
        .expect("folds");
    let mut twin = ship();
    twin.member_index = 9;
    assert_eq!(
        state.fold_payload(&c, &payload(0x400, &[ship().bytes(), twin.bytes()])),
        Ok(FoldOutcome::DuplicateMemberIdentity),
        "duplicate identity is checked before resurrection"
    );
}

// --- The D49 parent mirror ----------------------------------------------------

#[test]
fn the_parent_mirror_reproduces_d49s_three_non_folding_paths_in_order() {
    let mut state = State::new();
    // Status bit 11 alone masks the hour — and it is checked BEFORE the failure
    // status, so a record carrying both reports the location/region path.
    assert_eq!(
        state.fold_lifecycle_payload(
            &lifecycle_payload(
                D49_STATUS_LOCATION_CANDIDATE_UNVALIDATED,
                &[lifecycle(1, 1)]
            ),
            true,
            ParentPolicy::FoldFaithful
        ),
        Ok(LifecycleOutcome::LocationRegionIncomplete)
    );
    assert!(state.parents().is_empty());
    // A live row missing either promoted field does the same.
    for cleared in [D49_LOCATION_PRESENT, D49_REGION_PRESENT] {
        let mut row = lifecycle(1, 1);
        row.flags &= !cleared;
        if cleared == D49_LOCATION_PRESENT {
            row.location_id = 0;
        } else {
            row.region_id = 0;
        }
        assert_eq!(
            state.fold_lifecycle_payload(&lifecycle_payload(0, &[row]), false, ParentPolicy::FoldFaithful),
            Ok(LifecycleOutcome::LocationRegionIncomplete)
        );
    }
    // A DELETE row may omit both (`world_fold.rs:20051`).
    assert_eq!(
        state.fold_lifecycle_payload(&lifecycle_payload(0, &[lifecycle_delete(1, 1)]), false, ParentPolicy::FoldFaithful),
        Ok(LifecycleOutcome::Applied)
    );
    // The failure status is next.
    assert_eq!(
        state.fold_lifecycle_payload(&lifecycle_payload(0x200, &[lifecycle(1, 1)]), true, ParentPolicy::FoldFaithful),
        Ok(LifecycleOutcome::FailureStatus)
    );
    assert!(state.parents().is_empty());
    // Then the four side scalars, each independently.
    for cleared in [
        D49_ATTACKER_POSITIONING_PRESENT,
        D49_DEFENDER_POSITIONING_PRESENT,
        D49_ATTACKER_ANTI_AIR_PRESENT,
        D49_DEFENDER_ANTI_AIR_PRESENT,
    ] {
        let mut row = lifecycle(1, 1);
        row.flags &= !cleared;
        match cleared {
            D49_ATTACKER_POSITIONING_PRESENT => row.attacker_positioning_raw = 0,
            D49_DEFENDER_POSITIONING_PRESENT => row.defender_positioning_raw = 0,
            D49_ATTACKER_ANTI_AIR_PRESENT => row.attacker_anti_air_raw = 0,
            _ => row.defender_anti_air_raw = 0,
        }
        assert_eq!(
            state.fold_lifecycle_payload(&lifecycle_payload(0, &[row]), false, ParentPolicy::FoldFaithful),
            Ok(LifecycleOutcome::IncompleteSideScalars),
            "cleared {cleared:#x}"
        );
        assert!(state.parents().is_empty());
    }
    // And a well-formed payload lands.
    assert_eq!(
        state.fold_lifecycle_payload(&lifecycle_payload(0x400, &[lifecycle(1, 1)]), false, ParentPolicy::FoldFaithful),
        Ok(LifecycleOutcome::Applied)
    );
    assert_eq!(state.parents().len(), 1);
}

#[test]
fn the_identity_only_policy_relaxes_the_value_gates_and_nothing_else() {
    // The measured shape of `runs/batch3/deepwar_a`: every live D49 row omits the
    // promoted location/region pair, so the fold-faithful policy folds none of
    // them and d50's whole capture becomes orphan-refused. `IdentityOnly` admits
    // the combat IDENTITY — the only thing d50's orphan refusal reads — while
    // changing nothing else.
    let mut row = lifecycle(1, 1);
    row.flags &= !(D49_LOCATION_PRESENT | D49_REGION_PRESENT);
    row.location_id = 0;
    row.region_id = 0;

    let mut faithful = State::new();
    assert_eq!(
        faithful.fold_lifecycle_payload(&lifecycle_payload(0, &[row]), false, ParentPolicy::FoldFaithful),
        Ok(LifecycleOutcome::LocationRegionIncomplete)
    );
    assert!(faithful.parents().is_empty());

    let mut relaxed = State::new();
    assert_eq!(
        relaxed.fold_lifecycle_payload(&lifecycle_payload(0, &[row]), false, ParentPolicy::IdentityOnly),
        Ok(LifecycleOutcome::AppliedOverValueGate),
        "the outcome names the divergence rather than hiding it"
    );
    assert_eq!(relaxed.parents().len(), 1);
    // The lane-local status badge is relaxed the same way ...
    let mut badged = State::new();
    assert_eq!(
        badged.fold_lifecycle_payload(
            &lifecycle_payload(D49_STATUS_LOCATION_CANDIDATE_UNVALIDATED, &[lifecycle(1, 1)]),
            false,
            ParentPolicy::IdentityOnly
        ),
        Ok(LifecycleOutcome::AppliedOverValueGate)
    );
    // ... and so is the side-scalar gate, which is the same class of statement
    // about D49's own values.
    let mut no_scalars = lifecycle(1, 1);
    no_scalars.flags &= !D49_ATTACKER_ANTI_AIR_PRESENT;
    no_scalars.attacker_anti_air_raw = 0;
    let mut scalars = State::new();
    assert_eq!(
        scalars.fold_lifecycle_payload(&lifecycle_payload(0, &[no_scalars]), false, ParentPolicy::IdentityOnly),
        Ok(LifecycleOutcome::AppliedOverValueGate)
    );
    assert_eq!(scalars.parents().len(), 1);
    // A failure status is NOT relaxed: it is a statement about the capture, not
    // about D49's value promotion.
    let mut failed = State::new();
    assert_eq!(
        failed.fold_lifecycle_payload(&lifecycle_payload(0x200, &[lifecycle(1, 1)]), true, ParentPolicy::IdentityOnly),
        Ok(LifecycleOutcome::FailureStatus)
    );
    assert!(failed.parents().is_empty());
    // Nor is a payload the lane cannot decode.
    let mut bad = lifecycle(1, 1);
    bad.reserved_tail = 1;
    assert!(State::new()
        .fold_lifecycle_payload(&lifecycle_payload(0, &[bad]), false, ParentPolicy::IdentityOnly)
        .is_err());
    // And a well-formed row still reports the exact `Applied`, so the two
    // outcomes stay distinguishable in a harness's counters.
    let mut clean = State::new();
    assert_eq!(
        clean.fold_lifecycle_payload(&lifecycle_payload(0, &[lifecycle(1, 1)]), false, ParentPolicy::IdentityOnly),
        Ok(LifecycleOutcome::Applied)
    );
    // The default is the recorder's own policy — a caller that forgets to choose
    // gets the truth, not the convenient number.
    assert_eq!(ParentPolicy::default(), ParentPolicy::FoldFaithful);
}

#[test]
fn a_parent_delete_cascades_into_this_domains_members() {
    // `world_fold.rs:20098` removes the whole NavalCombatFoldState, whose
    // `members` map goes with it. This is the second writer of d50's own
    // accumulator, and it is not inert on any capture with a combat that ends.
    let c = collection();
    let mut state = state_with_parent(165_521, 7);
    state
        .fold_lifecycle_payload(&lifecycle_payload(0, &[lifecycle(165_522, 8)]), false, ParentPolicy::FoldFaithful)
        .expect("folds");
    let mut other_combat = ship();
    other_combat.combat_id = 165_522;
    other_combat.generation = 8;
    state
        .fold_payload(
            &c,
            &payload(0x400, &[ship().bytes(), other_combat.bytes()]),
        )
        .expect("folds");
    assert_eq!(state.members.len(), 2);

    state
        .fold_lifecycle_payload(&lifecycle_payload(0, &[lifecycle_delete(165_521, 7)]), false, ParentPolicy::FoldFaithful)
        .expect("folds");
    assert_eq!(state.parents().len(), 1, "the parent key is gone");
    assert_eq!(
        state.members.len(),
        1,
        "and so is every member under it — but only under it"
    );
    assert!(state
        .members
        .contains_key(&member_key(165_522, 8, SIDE_ATTACKER, 11)));
    // A member for the deleted combat is now an orphan, which refuses the hour.
    assert_eq!(
        state.fold_payload(&c, &payload(0x400, &[ship().bytes()])),
        Ok(FoldOutcome::OrphanMemberReference)
    );
    // The cascade does NOT retire the keys: only a d50 delete row does
    // (`world_fold.rs:20204`).
    assert!(state.retired().is_empty());
}

#[test]
fn parent_deletes_are_counted_per_row_not_by_the_net_set_size() {
    // One payload that inserts one combat and deletes another leaves the parent
    // set the same size. A counter derived from that size reports zero deletions
    // and zero cascaded members; the fold performed one of each.
    let c = collection();
    let mut state = state_with_parent(165_521, 7);
    state
        .fold_payload(&c, &payload(0x400, &[ship().bytes()]))
        .expect("folds");
    assert_eq!(state.members.len(), 1);
    let parents_before = state.parents().len();

    state
        .fold_lifecycle_payload(
            &lifecycle_payload(
                0,
                &[lifecycle_delete(165_521, 7), lifecycle(165_522, 8)],
            ),
            false,
            ParentPolicy::FoldFaithful,
        )
        .expect("folds");

    assert_eq!(
        state.parents().len(),
        parents_before,
        "the net size is unchanged, which is exactly the trap"
    );
    assert_eq!(state.parent_deletes_applied(), 1);
    assert_eq!(state.members_dropped_by_parent_delete(), 1);
    assert!(state.members.is_empty());

    // A tombstone for a combat that is not held counts nothing.
    state
        .fold_lifecycle_payload(
            &lifecycle_payload(0, &[lifecycle_delete(999_999, 1)]),
            false,
            ParentPolicy::FoldFaithful,
        )
        .expect("folds");
    assert_eq!(state.parent_deletes_applied(), 1);
    assert_eq!(state.members_dropped_by_parent_delete(), 1);

    // Two tombstones in one payload count twice, and both cascades are summed.
    let mut two = state_with_parent(1, 1);
    two.fold_lifecycle_payload(
        &lifecycle_payload(0, &[lifecycle(2, 1)]),
        false,
        ParentPolicy::FoldFaithful,
    )
    .expect("folds");
    let mut a = ship();
    a.combat_id = 1;
    a.generation = 1;
    let mut b = ship();
    b.combat_id = 2;
    b.generation = 1;
    two.fold_payload(&c, &payload(0x400, &[a.bytes(), b.bytes()]))
        .expect("folds");
    assert_eq!(two.members.len(), 2);
    two.fold_lifecycle_payload(
        &lifecycle_payload(0, &[lifecycle_delete(1, 1), lifecycle_delete(2, 1)]),
        false,
        ParentPolicy::FoldFaithful,
    )
    .expect("folds");
    assert_eq!(two.parent_deletes_applied(), 2);
    assert_eq!(two.members_dropped_by_parent_delete(), 2);
    assert!(two.parents().is_empty());
    assert!(two.members.is_empty());
}

#[test]
fn a_re_emitted_parent_row_keeps_its_members() {
    // `or_insert_with` (`world_fold.rs:20101`-`:20113`) leaves an existing
    // combat's member map alone, so a live d49 row for a combat already held
    // must not disturb this domain's state.
    let c = collection();
    let mut state = state_with_parent(165_521, 7);
    state
        .fold_payload(&c, &payload(0x400, &[ship().bytes()]))
        .expect("folds");
    let members_before = state.members.clone();
    state
        .fold_lifecycle_payload(&lifecycle_payload(0, &[lifecycle(165_521, 7)]), false, ParentPolicy::FoldFaithful)
        .expect("folds");
    assert_eq!(state.members, members_before);
}

#[test]
fn the_parent_mirror_rejects_every_lifecycle_row_the_lane_rejects() {
    // A malformed d49 payload is a fold `Err` — a stream rejection, not an
    // excluded hour — because `decode_lifecycle_rows` runs before every gate.
    for bit in 9..32u32 {
        let mut row = lifecycle(1, 1);
        row.flags |= 1 << bit;
        assert!(
            State::new()
                .fold_lifecycle_payload(&lifecycle_payload(0, &[row]), false, ParentPolicy::FoldFaithful)
                .is_err(),
            "flag bit {bit}"
        );
    }
    for id in [0u32, MAX_COMBAT_ID + 1] {
        let mut row = lifecycle(1, 1);
        row.combat_id = id;
        assert!(State::new()
            .fold_lifecycle_payload(&lifecycle_payload(0, &[row]), false, ParentPolicy::FoldFaithful)
            .is_err());
    }
    // Positioning is admitted only in [0, 100000] (lane `:1935`-`:1949`).
    for value in [-1i32, D49_MAX_POSITIONING_RAW + 1] {
        let mut row = lifecycle(1, 1);
        row.attacker_positioning_raw = value;
        assert!(State::new()
            .fold_lifecycle_payload(&lifecycle_payload(0, &[row]), false, ParentPolicy::FoldFaithful)
            .is_err());
    }
    // The trailing reserved word, and a gated-off word carrying data.
    let mut reserved = lifecycle(1, 1);
    reserved.reserved_tail = 1;
    assert!(State::new()
        .fold_lifecycle_payload(&lifecycle_payload(0, &[reserved]), false, ParentPolicy::FoldFaithful)
        .is_err());
    let mut ungated = lifecycle_delete(1, 1);
    ungated.location_id = 5;
    assert!(State::new()
        .fold_lifecycle_payload(&lifecycle_payload(0, &[ungated]), false, ParentPolicy::FoldFaithful)
        .is_err());
    // The lifecycle payload declares its own row length, so a member payload
    // handed to the lifecycle mirror is rejected rather than mis-sliced.
    assert!(State::new()
        .fold_lifecycle_payload(&payload(0, &[ship().bytes()]), false, ParentPolicy::FoldFaithful)
        .is_err());
}

// --- The delta grammar the measurement rests on -------------------------------

#[test]
fn an_unchanged_member_produces_no_delta_row() {
    // Stored equality is what the differ asks, and it is what makes a
    // measurement of "changed rows" mean anything. A re-emitted identical row
    // must not show up as churn.
    let c = collection();
    let mut a = state_with_parent(165_521, 7);
    a.fold_payload(&c, &payload(0x400, &[ship().bytes()]))
        .expect("folds");
    let first = a.members.clone();
    a.fold_payload(&c, &payload(0x400, &[ship().bytes()]))
        .expect("folds");
    assert!(diff(&c, &first, &a.members).is_empty());

    // A change below the stored grid of `Fixed(5)` does not exist: the source is
    // an integer over 10^5, so every distinct raw word is a distinct stored
    // value. One unit of strength is a delta.
    let mut moved = ship();
    moved.strength_raw += 1;
    a.fold_payload(&c, &payload(0x400, &[moved.bytes()]))
        .expect("folds");
    let d = diff(&c, &first, &a.members);
    assert_eq!(d.changed.len(), 1);
    assert!(d.tombstones.is_empty());
}
