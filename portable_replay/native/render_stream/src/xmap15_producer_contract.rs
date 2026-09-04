//! Who owes every mandatory leaf of the XMAP15 orders/fronts/movement sidecar.
//!
//! The carrier `order_of_battle.extra.xmap15_orders_fronts_movement_semantic`
//! is read every hour by `map_viewer/src/production_features/xmap15.rs` and no
//! extractor writes it. Lane WL classified its two ROW types leaf by leaf into
//! pinned and unsourced; this module keeps that classification, adds the
//! ENVELOPE it did not cover, and gives every gate an owner and a closing step
//! in the shared vocabulary of [`crate::carrier_producer_contract`].
//!
//! # The one thing that makes this family different from XMAP13
//!
//! XMAP13's gaps are things the producer cannot READ. XMAP15's movement census
//! is something the producer cannot honestly SAY. `Xmap15SemanticFrame::
//! validate` requires `omitted_movement_rows == 0` and
//! `movement_object_census == movements.len()`, so a frame carrying an empty
//! movement list is not a smaller census - it is the positive claim that the
//! engine had no movement arrow objects this hour. Nothing recorded says which
//! divisions owe a movement row, so a producer emitting zero would be
//! asserting a measurement it never made. That is why the carrier's declared
//! absence is `OmitKey` and not an empty frame.

use crate::carrier_producer_contract::{
    validate_carrier_contract_table, CarrierGateOwner, CarrierHourGate, CarrierLeafSource,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// The producer contract these entries classify.
pub const XMAP15_PRODUCER_CONTRACT_SCHEMA: &str = "hoi4.xmap15-orders-fronts-movement-semantic.v1";

/// The carrier's wire path, from the root of the folded hour.
pub const XMAP15_CARRIER_WIRE_PATH: [&str; 3] = [
    "order_of_battle",
    "extra",
    "xmap15_orders_fronts_movement_semantic",
];

/// The per-hour producer gate's wire path, beside the carrier's own key.
pub const XMAP15_PRODUCER_GATE_WIRE_PATH: [&str; 3] = [
    "order_of_battle",
    "extra",
    "xmap15_orders_fronts_movement_semantic_gate",
];

pub const XMAP15_PRODUCER_GATE_SCHEMA: &str = "hoi4.xmap15.producer-gate.v1";

/// The exact d54 order row `replay_core::army_battleplans::parse_order` admits,
/// spelled ONCE for both sides of the census.
///
/// `cross_check_d54_and_units` requires the producer's plan count to EQUAL the
/// consumer's visual d54 count, so a bound that differs by one on either side is
/// a census mismatch that names neither the row nor the field that differed -
/// and the two sides live in workspaces that cannot depend on each other, so a
/// second spelling drifts silently. It drifted once: the producer allowed order
/// instance ids to `i32::MAX` while the consumer stopped at a hundred million,
/// so a row above that was minted here and made the whole view unavailable
/// there. `army_battleplans.rs` takes its own constants from these.
pub const XMAP15_D54_ORDER_ROW_FIELDS: [&str; 18] = [
    "army_id",
    "engine_order_instance_id",
    "generation",
    "slot_index",
    "order_type",
    "target_states",
    "path_provinces",
    "target_declared_count",
    "target_status",
    "geometry_status",
    "execution_status",
    "assigned_division_ids",
    "assigned_units_status",
    "order_linkage_status",
    "active",
    "active_status",
    "style_flags",
    "style_status",
];

/// on to pick an order's `gfx/maparrows/maparrows.txt` definition, one bit each
/// in that function's own branch order. The engine reads them from the order at
/// +0x118, +0x119, +0x298, +0x299 and +0x378, and from its owner at +0x39.
pub const XMAP15_D54_STYLE_MOVE_SPECIAL: u32 = 1;
pub const XMAP15_D54_STYLE_WITHDRAW: u32 = 1 << 1;
pub const XMAP15_D54_STYLE_FALLBACK: u32 = 1 << 2;
pub const XMAP15_D54_STYLE_VIRTUAL: u32 = 1 << 3;
pub const XMAP15_D54_STYLE_OWNER_OVERRIDE: u32 = 1 << 4;
pub const XMAP15_D54_STYLE_OWNER_GROUPED: u32 = 1 << 5;
pub const XMAP15_D54_STYLE_FLAG_MASK: u32 = XMAP15_D54_STYLE_MOVE_SPECIAL
    | XMAP15_D54_STYLE_WITHDRAW
    | XMAP15_D54_STYLE_FALLBACK
    | XMAP15_D54_STYLE_VIRTUAL
    | XMAP15_D54_STYLE_OWNER_OVERRIDE
    | XMAP15_D54_STYLE_OWNER_GROUPED;

pub const XMAP15_D54_MAX_ARMY_ID: u32 = i32::MAX as u32;
pub const XMAP15_D54_MAX_ORDER_INSTANCE_ID: u32 = 100_000_000;
pub const XMAP15_D54_MAX_MAP_ID: u32 = 65_535;
pub const XMAP15_D54_SLOT_INDEX_COUNT: u32 = 64;

/// The recorded inputs this producer reads.
///
/// Three, and they are the whole of what a capture can give this family. d54's
/// battleplan orders are the plan census the consumer requires this one to
/// agree with row for row. `units.divisions` is there because every assigned
/// division has to appear exactly once in the XMAP13 identity census, and
/// because an ARMY plan owner's country is the country of the divisions
/// assigned to that army. `orders.theatre_army_hierarchy` is there for the
/// narrower half of the same field: an ARMY GROUP owner's country is not in d54
/// at all, so it comes from the theatre selector by way of the XMAP13 formation
/// census the consumer matches this row's owner against.
pub const XMAP15_RECORDED_INPUTS: [&str; 3] = [
    "orders.army_battleplans_semantic",
    "orders.theatre_army_hierarchy",
    "units.divisions",
];

/// The domain whose capture shape decides the unobservable half.
pub const XMAP15_BLOCKING_DOMAIN_ID: u16 = 77;

/// Every mandatory leaf of `Xmap15SemanticFrame`.
pub const XMAP15_ENVELOPE_CONTRACT: [(&str, CarrierLeafSource); 13] = [
    (
        "schema",
        CarrierLeafSource::RecordedPlayTimeState {
            recorded_input: "the producer's own contract token",
        },
    ),
    (
        "executable_sha256",
        CarrierLeafSource::RecordedPlayTimeState {
            recorded_input: "the attached executable's measured digest",
        },
    ),
    (
        "producer_registration_sha256",
        CarrierLeafSource::RecordedPlayTimeState {
            recorded_input: "the producer's own registration digest, which \
                             m03_arrows.rs::Xmap15SemanticFrame::validate pins to \
                             xmap15_semantic_producer_registration_sha256",
        },
    ),
    (
        "hour_index",
        CarrierLeafSource::RecordedPlayTimeState {
            recorded_input: "world.calendar_time; the folded hour's own index",
        },
    ),
    (
        "observation_sequence",
        CarrierLeafSource::RecordedPlayTimeState {
            recorded_input: "the producer's own observation counter; validate refuses zero",
        },
    ),
    (
        "baseline_complete",
        CarrierLeafSource::RecordedPlayTimeState {
            recorded_input: "the producer's own statement about its walk; validate refuses false",
        },
    ),
    (
        "omitted_plan_rows",
        CarrierLeafSource::RecordedPlayTimeState {
            recorded_input: "the producer's own omission count; validate refuses anything but zero",
        },
    ),
    (
        "omitted_movement_rows",
        CarrierLeafSource::RecordedPlayTimeState {
            recorded_input: "the producer's own omission count; validate refuses anything but zero",
        },
    ),
    (
        "plan_object_census",
        CarrierLeafSource::RecordedPlayTimeState {
            recorded_input: "validate requires it to equal plans.len(), so it follows the plan \
                             vector and is not an independent measurement",
        },
    ),
    (
        "movement_object_census",
        CarrierLeafSource::RecordedPlayTimeState {
            recorded_input: "validate requires it to equal movements.len(), so it follows the \
                             movement vector - which does not make the movement vector itself \
                             sourceable; see the movements entry",
        },
    ),
    (
        "plans",
        CarrierLeafSource::RecordedPlayTimeState {
            recorded_input: "the visual d54 orders - owner kind Army or ArmyGroup, order types 1 \
                             through 4 - which xmap15.rs::cross_check_d54_and_units requires the \
                             plan census to equal in COUNT and then match row for row. The row's \
                             own leaves are not all recorded; see XMAP15_PLAN_ROW_CONTRACT",
        },
    ),
    (
        "movements",
        CarrierLeafSource::Gate {
            owed_by: CarrierGateOwner::Producer,
            missing_input: "which divisions owe a movement row. This is the leaf that makes the \
                            family unemittable rather than merely incomplete: validate requires \
                            omitted_movement_rows == 0 and movement_object_census == \
                            movements.len(), so an empty movement vector is not a smaller census \
                            but the positive claim that the engine had no movement arrow objects \
                            this hour. No recorded domain carries that census - \
                            cross_check_d54_and_units touches movement rows once, only to require \
                            that a named division exists in the XMAP13 census, and no d54 order \
                            corresponds to a movement row at all",
            closing_step: "observe the engine's movement arrow-object enumeration, which is the \
                           same render session domain 77 needs; until then no honest frame exists \
                           to emit, which is why this carrier's declared absence omits the key \
                           rather than writing an empty one",
        },
    ),
    (
        "enumerator_witness_sha256",
        CarrierLeafSource::Gate {
            owed_by: CarrierGateOwner::Producer,
            missing_input: "a digest over the engine's own arrow-object enumeration. It exists to \
                            make the census claim above checkable, so it cannot be sourced from \
                            anything other than that enumeration",
            closing_step: "same observer as the movement census; the two close together or not at \
                           all",
        },
    ),
];

/// Every mandatory leaf of `Xmap15PlanSemanticRow`.
pub const XMAP15_PLAN_ROW_CONTRACT: [(&str, CarrierLeafSource); 38] = [
    (
        "key.owner_kind",
        CarrierLeafSource::RecordedPlayTimeState {
            recorded_input: "d54 order key owner kind, matched by cross_check_d54_and_units",
        },
    ),
    (
        "key.owner_id",
        CarrierLeafSource::RecordedPlayTimeState {
            recorded_input: "d54 order key owner id",
        },
    ),
    (
        "key.engine_order_instance_id",
        CarrierLeafSource::RecordedPlayTimeState {
            recorded_input: "d54 order key engine_order_instance_id",
        },
    ),
    (
        "key.generation",
        CarrierLeafSource::RecordedPlayTimeState {
            recorded_input: "d54 order key generation",
        },
    ),
    (
        "key.slot_index",
        CarrierLeafSource::RecordedPlayTimeState {
            recorded_input: "d54 order slot_index, compared exactly",
        },
    ),
    (
        "owner_country",
        CarrierLeafSource::RecordedPlayTimeState {
            recorded_input: "the country of the XMAP13 formation this owner names: \
                             formation_identity_match_count requires exactly one formation for \
                             the (owner kind, owner id, country) triple, and formations are \
                             unique by kind and id, so only the recorded country satisfies it",
        },
    ),
    (
        "ordered_assigned_division_ids",
        CarrierLeafSource::RecordedPlayTimeState {
            recorded_input: "d54 assigned_division_ids, compared as an ordered vector",
        },
    ),
    (
        "assigned",
        CarrierLeafSource::RecordedPlayTimeState {
            recorded_input: "validate ties it to whether ordered_assigned_division_ids is empty, \
                             so it has no independent value to get wrong",
        },
    ),
    (
        "active",
        CarrierLeafSource::RecordedPlayTimeState {
            recorded_input: "d54 order active flag, compared exactly",
        },
    ),
    (
        "route.ordered_province_ids",
        CarrierLeafSource::RecordedPlayTimeState {
            recorded_input: "d54 plan geometry - the path provinces, or the source and target of \
                             a paradrop - compared exactly by cross_check_d54_and_units",
        },
    ),
    (
        "selector.order_type",
        CarrierLeafSource::RecordedPlayTimeState {
            recorded_input: "d54 order_type, compared exactly",
        },
    ),
    (
        "resolved_definition_name",
        CarrierLeafSource::ReplayDerived {
            authority: "Xmap15PlanDefinitionSelector::resolved_definition_name computes it from \
                        the selector, and Xmap15SemanticFrame::validate REFUSES a row whose \
                        recorded name is anything else - so a producer sourcing it would be a \
                        second writer on a fact the frame's own validator already derives. Lane \
                        WL counted it as unsourced because its INPUT is; that is true of the \
                        input and not of this leaf",
        },
    ),
    ("selector.move_special_118", ORDER_FLAG_GATE),
    ("selector.withdraw_119", ORDER_FLAG_GATE),
    ("selector.fallback_298", ORDER_FLAG_GATE),
    ("selector.virtual_299", ORDER_FLAG_GATE),
    ("selector.blitz_118", ORDER_FLAG_GATE),
    ("selector.army_group_style_39_and_378", ORDER_FLAG_GATE),
    ("route.builder_input_points_fixed5", BUILDER_GEOMETRY_GATE),
    ("route.engine_pre_smoother_points_fixed5", BUILDER_GEOMETRY_GATE),
    ("route.point_width_classes", BUILDER_GEOMETRY_GATE),
    ("route.explicit_height_f32_bits", BUILDER_GEOMETRY_GATE),
    ("route.wrap_offset_world", BUILDER_GEOMETRY_GATE),
    ("exact_head_vertices_f32_bits", BUILDER_GEOMETRY_GATE),
    ("manager_insertion_ordinal", DRAW_ORDER_GATE),
    ("definition_group_ordinal", DRAW_ORDER_GATE),
    ("comparator_tie_17c_f32_bits", DRAW_ORDER_GATE),
    ("final_engine_draw_ordinal", DRAW_ORDER_GATE),
    ("object_width_multiplier_f32_bits", ARROW_APPEARANCE_GATE),
    ("fade_in_f32_bits", ARROW_APPEARANCE_GATE),
    ("fade_out_f32_bits", ARROW_APPEARANCE_GATE),
    ("culling_offset_f32_bits", ARROW_APPEARANCE_GATE),
    ("inner_rgba_f32_bits", ARROW_APPEARANCE_GATE),
    ("outer_rgba_f32_bits", ARROW_APPEARANCE_GATE),
    ("texture_variant", ARROW_APPEARANCE_GATE),
    ("see_through", ARROW_APPEARANCE_GATE),
    ("visible", ARROW_APPEARANCE_GATE),
    ("lifecycle_tombstone_sha256", LIFECYCLE_GATE),
];

/// Every mandatory leaf of `Xmap15MovementSemanticRow`. Not one is recorded.
pub const XMAP15_MOVEMENT_ROW_CONTRACT: [(&str, CarrierLeafSource); 29] = [
    ("key.owner_country", MOVEMENT_CENSUS_GATE),
    ("key.division_id", MOVEMENT_CENSUS_GATE),
    ("key.movement_generation", MOVEMENT_CENSUS_GATE),
    ("route.ordered_province_ids", MOVEMENT_CENSUS_GATE),
    ("definition", MOVEMENT_CENSUS_GATE),
    ("progress_raw_u32", MOVEMENT_CENSUS_GATE),
    ("progress_move_army_f32_bits", MOVEMENT_CENSUS_GATE),
    ("desaturation_lengths_f32_bits", MOVEMENT_CENSUS_GATE),
    ("offset_x_f32_bits", MOVEMENT_CENSUS_GATE),
    ("manager_insertion_ordinal", DRAW_ORDER_GATE),
    ("definition_group_ordinal", DRAW_ORDER_GATE),
    ("comparator_tie_17c_f32_bits", DRAW_ORDER_GATE),
    ("final_engine_draw_ordinal", DRAW_ORDER_GATE),
    ("object_width_multiplier_f32_bits", ARROW_APPEARANCE_GATE),
    ("fade_in_f32_bits", ARROW_APPEARANCE_GATE),
    ("fade_out_f32_bits", ARROW_APPEARANCE_GATE),
    ("inner_rgba_f32_bits", ARROW_APPEARANCE_GATE),
    ("outer_rgba_f32_bits", ARROW_APPEARANCE_GATE),
    ("texture_variant", ARROW_APPEARANCE_GATE),
    ("see_through", ARROW_APPEARANCE_GATE),
    ("visible", ARROW_APPEARANCE_GATE),
    ("lifecycle_tombstone_sha256", LIFECYCLE_GATE),
    ("exact_vertex_stride_bytes", SUBMITTED_MESH_GATE),
    ("exact_vertex_count", SUBMITTED_MESH_GATE),
    ("exact_vertex_bytes", SUBMITTED_MESH_GATE),
    ("exact_index_format_u32", SUBMITTED_MESH_GATE),
    ("exact_index_count", SUBMITTED_MESH_GATE),
    ("exact_index_bytes", SUBMITTED_MESH_GATE),
    ("exact_primitive_topology_u32", SUBMITTED_MESH_GATE),
];

/// The subset the viewer uploads and DRAWS verbatim.
///
/// Kept as its own list because it is the one part of this family for which no
/// plausible substitute exists: `map_viewer/src/xmap15_d3d_handoff.rs` uploads
/// these bytes into D3D buffers and issues `DrawIndexed(exact_index_count)` for
/// an indexed row or `Draw(exact_vertex_count)` for a non-indexed one, under
/// `exact_primitive_topology_u32`. They are the engine's submitted mesh rather
/// than a description of one.
pub const XMAP15_SUBMITTED_MESH_LEAVES: [&str; 7] = [
    "exact_vertex_stride_bytes",
    "exact_vertex_count",
    "exact_vertex_bytes",
    "exact_index_format_u32",
    "exact_index_count",
    "exact_index_bytes",
    "exact_primitive_topology_u32",
];

const ORDER_FLAG_GATE: CarrierLeafSource = CarrierLeafSource::Gate {
    owed_by: CarrierGateOwner::Producer,
    missing_input: "the engine's own order flag bits, which pick the arrow DEFINITION. d54 \
                    records the order type and nothing finer, and these six are what separate a \
                    move from a blitz, a withdraw, a fallback and an army-group style at the same \
                    order type",
    closing_step: "prove a passive read of the order object's flag words on the a729 image, or \
                   prove each flag is a function of state d54 already records - the latter would \
                   move these leaves to the viewer rather than to a producer",
};

const BUILDER_GEOMETRY_GATE: CarrierLeafSource = CarrierLeafSource::Gate {
    owed_by: CarrierGateOwner::Producer,
    missing_input: "the arrow builder's exact input geometry - the fixed-point control points it \
                    is handed, the pre-smoother points, the per-point width classes, any explicit \
                    heights, the longitude wrap offset and the built head vertices. The route's \
                    PROVINCES are recorded; none of this is, and it is the geometry the engine \
                    built rather than the path it built it from",
    closing_step: "observe the arrow object's builder inputs, which is the same render session \
                   the movement census needs; or prove the builder is a pure function of the \
                   province path and the map, which would move these to the viewer",
};

const DRAW_ORDER_GATE: CarrierLeafSource = CarrierLeafSource::Gate {
    owed_by: CarrierGateOwner::Producer,
    missing_input: "where this object sits in the engine's draw order among all arrow objects - \
                    its insertion ordinal in the manager, its ordinal within its definition \
                    group, the comparator tie-break and the final draw ordinal. \
                    Xmap15SemanticFrame::validate requires final_engine_draw_ordinal to be unique \
                    across plans AND movements together, so it is an ordering over a set that \
                    includes the census this producer cannot state",
    closing_step: "observe the arrow manager's ordering, or prove it is a total order over state \
                   the recording already carries - which cannot be shown while the movement half \
                   of the set is unobserved",
};

const ARROW_APPEARANCE_GATE: CarrierLeafSource = CarrierLeafSource::Gate {
    owed_by: CarrierGateOwner::Producer,
    missing_input: "the arrow object's runtime draw state - width multiplier, fade in and out, \
                    culling offset, inner and outer colours, texture variant, see-through and \
                    visibility. WHETHER these are authored on the arrow definition or computed at \
                    runtime is UNPROVEN: the consumer resolves the definition by name and \
                    validates only that the recorded values are finite, so nothing in the tree \
                    settles which side owns them",
    closing_step: "read the authored arrow definitions and compare them against a live frame's \
                   values. If they match, these leaves are the viewer's and leave the schema; if \
                   they do not, they are a producer read and the difference is what is owed. \
                   Recording them before that is answered would put a copy of authored content on \
                   the wire",
};

const LIFECYCLE_GATE: CarrierLeafSource = CarrierLeafSource::Gate {
    owed_by: CarrierGateOwner::Producer,
    missing_input: "the arrow object's lifecycle tombstone, a digest identifying the object \
                    instance across the frames it lives for. It is a property of an object this \
                    producer never observes",
    closing_step: "same observer as the census; a tombstone for an object nobody enumerated is \
                   not a value that can be derived",
};

const MOVEMENT_CENSUS_GATE: CarrierLeafSource = CarrierLeafSource::Gate {
    owed_by: CarrierGateOwner::Producer,
    missing_input: "the movement arrow census itself. cross_check_d54_and_units requires a \
                    movement row's division to appear exactly once in the XMAP13 identity census, \
                    which says a named division exists - it does not say WHICH divisions owe a \
                    row, and no d54 order corresponds to a movement row. The row's own state - \
                    the generation, the definition, the progress words, the desaturation lengths, \
                    the offset and the route - is arrow-object runtime state on top of that",
    closing_step: "observe the movement arrow objects. Note what does NOT close it: a division's \
                   recorded move order. DivisionUnitRow carries location_province, \
                   move_target_province and move_progress, which is one hop rather than the route \
                   an arrow draws, and nothing proves a moving division has an arrow object or \
                   that a stationary one does not",
};

const SUBMITTED_MESH_GATE: CarrierLeafSource = CarrierLeafSource::Gate {
    owed_by: CarrierGateOwner::Producer,
    missing_input: "the engine's SUBMITTED MESH, not a description of one: \
                    map_viewer/src/xmap15_d3d_handoff.rs uploads these bytes into D3D buffers and \
                    draws them verbatim. The only observer of a submitted mesh is the render \
                    session compact_contract::EARLY_LAUNCH_ONLY_PRODUCERS records as held off an \
                    attach capture by three independent gates",
    closing_step: "the same three gates: the early-render control word, the bootstrap hooks that \
                   swap import slots a running game has already called, and the shipped \
                   bootstrap-only profile that admits zero staged hooks. All three, or this stays \
                   unobservable on every capture shape",
};

/// A recorded input this producer needs that one hour of one recording did not
/// carry.
pub type Xmap15HourGate = CarrierHourGate;

/// What the producer got out of one hour, and what it could not.
///
/// Deliberately not the same struct as XMAP13's: the two families count
/// different things, and a shared struct would have to carry both families'
/// census fields and leave the other family's at zero, which reads as a
/// measured zero.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap15ProducerGate {
    pub schema: String,
    pub hour_index: i64,
    /// Plan rows whose recorded leaves the producer minted this hour.
    pub minted_plan_rows: u32,
    /// The visual d54 orders the plan census must equal. Reported beside the
    /// mint because a mint short of it is a different fact from an hour with no
    /// visual orders.
    pub visual_d54_orders: u32,
    pub hour_gates: Vec<Xmap15HourGate>,
}

impl Xmap15ProducerGate {
    /// Always false. The movement census cannot be stated honestly by any
    /// producer that has not observed the arrow objects, so no hour of any
    /// recording seals this carrier.
    pub fn could_seal(&self) -> bool {
        self.hour_gates.is_empty() && xmap15_contract_gate_leaves().is_empty()
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema != XMAP15_PRODUCER_GATE_SCHEMA {
            return Err(format!("XMAP15 producer gate schema is {}", self.schema));
        }
        let mut seen = BTreeSet::new();
        for gate in &self.hour_gates {
            if !XMAP15_RECORDED_INPUTS.contains(&gate.recorded_input.as_str()) {
                return Err(format!(
                    "XMAP15 producer gate names {}, which is not one of the two recorded inputs \
                     this producer reads",
                    gate.recorded_input
                ));
            }
            if !seen.insert(gate.recorded_input.as_str()) {
                return Err(format!(
                    "XMAP15 producer gate names {} twice",
                    gate.recorded_input
                ));
            }
            if gate.recorded_state.trim().is_empty() {
                return Err(format!(
                    "XMAP15 producer gate on {} does not say what the recording said",
                    gate.recorded_input
                ));
            }
        }
        Ok(())
    }

    /// The whole cause, in one sentence, for a consumer reporting why the
    /// family is unavailable this hour.
    pub fn unavailable_reason(&self) -> String {
        let gates = xmap15_contract_gate_leaves();
        let hour = if self.hour_gates.is_empty() {
            "every recorded input this producer reads was carried this hour".to_owned()
        } else {
            format!(
                "this hour did not carry {}",
                self.hour_gates
                    .iter()
                    .map(|gate| format!("{} ({})", gate.recorded_input, gate.recorded_state))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        format!(
            "the XMAP15 producer minted the recorded half of {} plan rows against {} visual d54 \
             orders for hour {}; {}; and {} mandatory leaves have no source in any capture, of \
             which the movement census is the one that makes an empty frame a false claim rather \
             than a small one",
            self.minted_plan_rows,
            self.visual_d54_orders,
            self.hour_index,
            hour,
            gates.len()
        )
    }
}

/// Every contract-gated leaf across the three tables, in declaration order.
///
/// Deduplicated: the plan and movement rows share several gates by name, and a
/// count that reported `manager_insertion_ordinal` twice would be counting
/// tables rather than obligations.
pub fn xmap15_contract_gate_leaves() -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for (table, prefix) in [
        (XMAP15_ENVELOPE_CONTRACT.as_slice(), ""),
        (XMAP15_PLAN_ROW_CONTRACT.as_slice(), "plans[]."),
        (XMAP15_MOVEMENT_ROW_CONTRACT.as_slice(), "movements[]."),
    ] {
        for (path, source) in table {
            if source.gate_owner().is_none() {
                continue;
            }
            let qualified = format!("{prefix}{path}");
            if seen.insert(qualified.clone()) {
                out.push(qualified);
            }
        }
    }
    out
}

/// Every plan-row leaf the producer mints.
pub fn xmap15_minted_plan_leaves() -> Vec<&'static str> {
    XMAP15_PLAN_ROW_CONTRACT
        .iter()
        .filter(|(_, source)| source.is_minted_by_the_producer())
        .map(|(path, _)| *path)
        .collect()
}

pub fn xmap15_classify_envelope_leaf(path: &str) -> Option<&'static CarrierLeafSource> {
    classify(&XMAP15_ENVELOPE_CONTRACT, path)
}

pub fn xmap15_classify_plan_leaf(path: &str) -> Option<&'static CarrierLeafSource> {
    classify(&XMAP15_PLAN_ROW_CONTRACT, path)
}

pub fn xmap15_classify_movement_leaf(path: &str) -> Option<&'static CarrierLeafSource> {
    classify(&XMAP15_MOVEMENT_ROW_CONTRACT, path)
}

fn classify<'a>(
    table: &'a [(&'static str, CarrierLeafSource)],
    path: &str,
) -> Option<&'a CarrierLeafSource> {
    table
        .iter()
        .find(|(entry, _)| {
            *entry == path
                || path
                    .strip_prefix(*entry)
                    .is_some_and(|rest| rest.starts_with('.'))
        })
        .map(|(_, source)| source)
}

pub fn validate_xmap15_producer_contract() -> Result<(), String> {
    for (label, table) in [
        ("envelope", XMAP15_ENVELOPE_CONTRACT.as_slice()),
        ("plan row", XMAP15_PLAN_ROW_CONTRACT.as_slice()),
        ("movement row", XMAP15_MOVEMENT_ROW_CONTRACT.as_slice()),
    ] {
        validate_carrier_contract_table(label, table)?;
    }
    if XMAP15_RECORDED_INPUTS.iter().any(|id| id.trim().is_empty()) {
        return Err("a recorded input id is blank".into());
    }
    for leaf in XMAP15_SUBMITTED_MESH_LEAVES {
        match xmap15_classify_movement_leaf(leaf) {
            Some(CarrierLeafSource::Gate { .. }) => {}
            _ => {
                return Err(format!(
                    "{leaf} is a submitted-mesh leaf and must be gated: the viewer draws it \
                     verbatim, so no substitute exists"
                ))
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::{
        Xmap15MovementDefinition, Xmap15MovementKey, Xmap15MovementRoute, Xmap15MovementSemanticRow,
        Xmap15OwnerKind, Xmap15PlanDefinitionSelector, Xmap15PlanKey, Xmap15PlanSemanticRow,
        Xmap15PreparedRoute, Xmap15SemanticFrame,
    };

    #[test]
    fn the_contract_is_a_partition_and_every_gate_names_a_way_out() {
        validate_xmap15_producer_contract().expect("the producer contract must be well formed");
    }

    /// Every LEAF path of a serialized value, expanding nested objects only.
    fn leaf_paths(value: &serde_json::Value) -> BTreeSet<String> {
        fn walk(prefix: &str, value: &serde_json::Value, out: &mut BTreeSet<String>) {
            match value {
                serde_json::Value::Object(fields) => {
                    assert!(!fields.is_empty(), "{prefix} is an empty object");
                    for (name, child) in fields {
                        let path = if prefix.is_empty() {
                            name.clone()
                        } else {
                            format!("{prefix}.{name}")
                        };
                        walk(&path, child, out);
                    }
                }
                _ => {
                    assert!(out.insert(prefix.to_owned()), "{prefix} appears twice");
                }
            }
        }
        let mut out = BTreeSet::new();
        assert!(value.is_object(), "a semantic row serializes to an object");
        walk("", value, &mut out);
        out
    }

    fn plan_row() -> Xmap15PlanSemanticRow {
        // Every Option is `Some`: a `None` serializes to a null LEAF and would
        // let the composite under it go unclassified while this test passed.
        Xmap15PlanSemanticRow {
            key: Xmap15PlanKey {
                owner_kind: Xmap15OwnerKind::Army,
                owner_id: 1,
                engine_order_instance_id: 1,
                generation: 1,
                slot_index: 0,
            },
            owner_country: "GER".to_owned(),
            selector: Xmap15PlanDefinitionSelector {
                order_type: 1,
                move_special_118: false,
                withdraw_119: false,
                fallback_298: false,
                virtual_299: false,
                blitz_118: false,
                army_group_style_39_and_378: false,
            },
            resolved_definition_name: "move_thick".to_owned(),
            ordered_assigned_division_ids: vec![1],
            assigned: true,
            active: true,
            route: Xmap15PreparedRoute {
                ordered_province_ids: vec![1, 2],
                builder_input_points_fixed5: vec![[0, 0], [1, 1]],
                engine_pre_smoother_points_fixed5: Some(vec![[0, 0]]),
                point_width_classes: vec![0, 0],
                explicit_height_f32_bits: Some(vec![0]),
                wrap_offset_world: 0,
            },
            manager_insertion_ordinal: 0,
            definition_group_ordinal: 0,
            object_width_multiplier_f32_bits: 1.0f32.to_bits(),
            comparator_tie_17c_f32_bits: 0,
            fade_in_f32_bits: 0,
            fade_out_f32_bits: 0,
            culling_offset_f32_bits: 0,
            inner_rgba_f32_bits: [0; 4],
            outer_rgba_f32_bits: [0; 4],
            texture_variant: 0,
            see_through: false,
            visible: true,
            final_engine_draw_ordinal: 0,
            exact_head_vertices_f32_bits: Some([[0; 8]; 4]),
            lifecycle_tombstone_sha256: String::new(),
        }
    }

    fn movement_row() -> Xmap15MovementSemanticRow {
        Xmap15MovementSemanticRow {
            key: Xmap15MovementKey {
                owner_country: "GER".to_owned(),
                division_id: 1,
                movement_generation: 1,
            },
            definition: Xmap15MovementDefinition::MoveArrow,
            progress_raw_u32: 0,
            progress_move_army_f32_bits: [0; 3],
            desaturation_lengths_f32_bits: [0; 4],
            offset_x_f32_bits: 0,
            route: Xmap15MovementRoute {
                ordered_province_ids: vec![1],
            },
            manager_insertion_ordinal: 0,
            definition_group_ordinal: 0,
            object_width_multiplier_f32_bits: 1.0f32.to_bits(),
            comparator_tie_17c_f32_bits: 0,
            fade_in_f32_bits: 0,
            fade_out_f32_bits: 0,
            inner_rgba_f32_bits: [0; 4],
            outer_rgba_f32_bits: [0; 4],
            texture_variant: 0,
            see_through: false,
            visible: true,
            final_engine_draw_ordinal: 0,
            exact_vertex_stride_bytes: 0,
            exact_vertex_count: 0,
            exact_vertex_bytes: Vec::new(),
            exact_index_format_u32: 0,
            exact_index_count: 0,
            exact_index_bytes: Vec::new(),
            exact_primitive_topology_u32: 0,
            lifecycle_tombstone_sha256: String::new(),
        }
    }

    /// kills: the contract drifting off the schema it classifies, in either
    /// direction.
    ///
    /// A field added, renamed or removed anywhere in the frame or in either row
    /// type fails here until somebody says which of the three sources it has -
    /// and an entry that classifies a leaf the schema no longer carries fails
    /// too, because that is how a closed obligation stays on a blocker list.
    #[test]
    fn the_producer_contract_classifies_exactly_this_schema() {
        let frame = Xmap15SemanticFrame {
            schema: String::new(),
            executable_sha256: String::new(),
            producer_registration_sha256: String::new(),
            hour_index: 17,
            observation_sequence: 1,
            baseline_complete: true,
            omitted_plan_rows: 0,
            omitted_movement_rows: 0,
            plan_object_census: 1,
            movement_object_census: 1,
            plans: vec![plan_row()],
            movements: vec![movement_row()],
            enumerator_witness_sha256: String::new(),
        };
        for (label, leaves, classify) in [
            (
                "envelope",
                leaf_paths(&serde_json::to_value(&frame).expect("frame serializes")),
                xmap15_classify_envelope_leaf
                    as fn(&str) -> Option<&'static CarrierLeafSource>,
            ),
            (
                "plan row",
                leaf_paths(&serde_json::to_value(plan_row()).expect("plan row serializes")),
                xmap15_classify_plan_leaf as fn(&str) -> Option<&'static CarrierLeafSource>,
            ),
            (
                "movement row",
                leaf_paths(&serde_json::to_value(movement_row()).expect("movement row serializes")),
                xmap15_classify_movement_leaf as fn(&str) -> Option<&'static CarrierLeafSource>,
            ),
        ] {
            for leaf in &leaves {
                assert!(
                    classify(leaf).is_some(),
                    "the {label} leaf {leaf} has no entry in the producer contract"
                );
            }
            let table: &[(&str, CarrierLeafSource)] = match label {
                "envelope" => &XMAP15_ENVELOPE_CONTRACT,
                "plan row" => &XMAP15_PLAN_ROW_CONTRACT,
                _ => &XMAP15_MOVEMENT_ROW_CONTRACT,
            };
            for (entry, _) in table {
                assert!(
                    leaves.iter().any(|leaf| leaf == entry
                        || leaf
                            .strip_prefix(entry)
                            .is_some_and(|rest| rest.starts_with('.'))),
                    "the contract classifies the {label} entry {entry}, which the schema does \
                     not carry"
                );
            }
        }
    }

    /// kills: the counted split drifting off lane WL's landed classification.
    ///
    /// WL proved, against the serialized schema, that eleven plan leaves are
    /// pinned to recorded state and every movement leaf is not. This contract
    /// keeps that and moves exactly ONE leaf -- `resolved_definition_name`,
    /// which the frame's own validator derives from the selector -- from
    /// unsourced to the viewer's. Any other movement between the classes is a
    /// change of claim and has to be argued, not slipped in.
    #[test]
    fn the_counted_split_is_wls_plus_exactly_one_reclassification() {
        assert_eq!(xmap15_minted_plan_leaves().len(), 11);
        let derived: Vec<&str> = XMAP15_PLAN_ROW_CONTRACT
            .iter()
            .filter(|(_, source)| matches!(source, CarrierLeafSource::ReplayDerived { .. }))
            .map(|(path, _)| *path)
            .collect();
        assert_eq!(derived, ["resolved_definition_name"]);
        let plan_gates = XMAP15_PLAN_ROW_CONTRACT
            .iter()
            .filter(|(_, source)| source.gate_owner().is_some())
            .count();
        assert_eq!(plan_gates, 26);
        assert_eq!(
            XMAP15_MOVEMENT_ROW_CONTRACT
                .iter()
                .filter(|(_, source)| source.gate_owner().is_some())
                .count(),
            XMAP15_MOVEMENT_ROW_CONTRACT.len(),
            "not one movement leaf is recorded"
        );
    }

    /// kills: the movement census reading as "unbuilt" rather than
    /// "unstateable".
    ///
    /// It is the sharpest fact about this family. An empty movement vector
    /// validates -- `omitted_movement_rows == 0` and `movement_object_census ==
    /// movements.len()` are both satisfied by zero -- so a producer that
    /// emitted one would have made a positive claim about the engine it never
    /// observed. That is why the declared absence omits the key.
    #[test]
    fn the_movement_census_is_declared_unstateable_and_not_merely_missing() {
        let source = xmap15_classify_envelope_leaf("movements").expect("classified");
        let CarrierLeafSource::Gate { missing_input, .. } = source else {
            panic!("the movement census is a gate");
        };
        assert!(missing_input.contains("positive claim"), "{missing_input}");
        assert!(missing_input.contains("omitted_movement_rows == 0"), "{missing_input}");
        // And the counts that follow the vectors must NOT be gated with it, or
        // a reader takes the census question for an arithmetic one.
        for leaf in ["plan_object_census", "movement_object_census"] {
            assert!(matches!(
                xmap15_classify_envelope_leaf(leaf),
                Some(CarrierLeafSource::RecordedPlayTimeState { .. })
            ));
        }
    }

    /// kills: the submitted-mesh leaves losing the fact that the viewer DRAWS
    /// them, which is what rules out every substitute.
    #[test]
    fn the_submitted_mesh_leaves_are_gated_and_say_the_viewer_draws_them() {
        for leaf in XMAP15_SUBMITTED_MESH_LEAVES {
            let source = xmap15_classify_movement_leaf(leaf)
                .unwrap_or_else(|| panic!("{leaf} is classified"));
            let CarrierLeafSource::Gate { missing_input, .. } = source else {
                panic!("{leaf} must be gated");
            };
            assert!(missing_input.contains("draws them verbatim"), "{leaf}");
        }
        assert_eq!(XMAP15_BLOCKING_DOMAIN_ID, 77);
    }

    /// kills: the shared gates being counted once per table they appear in.
    #[test]
    fn the_gate_count_counts_obligations_and_not_table_rows() {
        let leaves = xmap15_contract_gate_leaves();
        let unique: BTreeSet<&String> = leaves.iter().collect();
        assert_eq!(unique.len(), leaves.len());
        // The plan and movement rows really do share gate names, so the
        // qualification is load-bearing rather than decorative.
        assert!(leaves.contains(&"plans[].manager_insertion_ordinal".to_owned()));
        assert!(leaves.contains(&"movements[].manager_insertion_ordinal".to_owned()));
    }

    #[test]
    fn a_gate_names_its_inputs_and_refuses_one_it_does_not_read() {
        let mut gate = Xmap15ProducerGate {
            schema: XMAP15_PRODUCER_GATE_SCHEMA.to_owned(),
            hour_index: 17,
            minted_plan_rows: 4,
            visual_d54_orders: 4,
            hour_gates: Vec::new(),
        };
        gate.validate().expect("a well-formed gate");
        assert!(!gate.could_seal());
        let reason = gate.unavailable_reason();
        assert!(reason.contains("4 plan rows against 4 visual"), "{reason}");
        assert!(reason.contains("false claim"), "{reason}");

        gate.hour_gates = vec![Xmap15HourGate {
            recorded_input: "units.wings".to_owned(),
            recorded_state: "anything".to_owned(),
        }];
        assert!(
            gate.validate().is_err(),
            "this producer never reads the wing census, so it may not gate on it"
        );
        gate.hour_gates[0].recorded_input = "units.divisions".to_owned();
        gate.validate().expect("an input this producer does read");
    }
}
