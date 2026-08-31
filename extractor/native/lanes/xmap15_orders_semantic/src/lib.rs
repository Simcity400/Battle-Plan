//! The XMAP15 semantic sidecar the viewer requires and no extractor writes.
//!
//! `order_of_battle.extra.xmap15_orders_fronts_movement_semantic` had no
//! declaration anywhere on the capture side: not a wave-2 carrier, not a
//! pending producer, not a fold arm. Its absence therefore read as "nobody has
//! looked", when what is true is narrower and worth counting.
//!
//! # The classification is exhaustive, and the tests hold it that way
//!
//! Every mandatory LEAF of the two semantic row types is in exactly one of
//! [`PLAN_LEAVES_PINNED_BY_RECORDED_STATE`] and [`PLAN_LEAVES_UNSOURCED`], or
//! in [`MOVEMENT_LEAVES_UNSOURCED`]. The tests build a row of each type,
//! serialize it, expand every nested object, and compare the classification
//! against the resulting leaf paths - so a field added anywhere in the schema
//! fails this crate until somebody says which half it is in. Leaves rather than
//! fields because a composite can be half-pinned: only
//! `route.ordered_province_ids` of a plan route is compared against domain 54,
//! and a field-level list said the whole route was.

use compact_contract::pending_producer::{
    PendingProducerCrate, PendingProducerSlot, RequiredSchema, WireAbsence,
};

/// The producer-side domain whose capture shape decides this carrier.
pub const BLOCKING_DOMAIN_ID: u16 = 77;

/// The domain that carries the plan half the consumer cross-checks.
pub const SEMANTIC_SOURCE_DOMAIN_ID: u16 = 54;

/// Plan-row LEAVES `xmap15.rs::cross_check_d54_and_units` pins to recorded
/// state, so a producer copies rather than measures them.
///
/// Leaves, not fields: the first review round found the whole `route` field
/// classified as sourced when only `route.ordered_province_ids` is compared,
/// and a field-level list cannot express that. Every composite is expanded, so
/// a subfield added to `Xmap15PlanKey`, `Xmap15PlanDefinitionSelector` or
/// `Xmap15PreparedRoute` has to be classified too.
///
/// `assigned` is here because the same check compares
/// `ordered_assigned_division_ids` to `d54.assigned_division_ids` and
/// `Xmap15SemanticFrame::validate` then ties `assigned` to whether that vector
/// is empty; it has no independent value to get wrong. `owner_country` is here
/// because `formation_identity_match_count` requires exactly one XMAP13
/// formation for the (owner kind, owner id, country) triple, and formation rows
/// are unique by kind and id, so only the recorded country satisfies it.
pub const PLAN_LEAVES_PINNED_BY_RECORDED_STATE: [&str; 11] = [
    "key.owner_kind",
    "key.owner_id",
    "key.engine_order_instance_id",
    "key.generation",
    "key.slot_index",
    "owner_country",
    "ordered_assigned_division_ids",
    "assigned",
    "active",
    "route.ordered_province_ids",
    "selector.order_type",
];

/// Plan-row leaves nothing recorded sources.
///
/// The selector's other six predicates are checked only for internal
/// consistency and are what picks the arrow definition, so
/// `resolved_definition_name` follows them rather than D54. The route's five
/// non-province leaves are the builder's exact input geometry, which no
/// recorded domain carries.
pub const PLAN_LEAVES_UNSOURCED: [&str; 27] = [
    "selector.move_special_118",
    "selector.withdraw_119",
    "selector.fallback_298",
    "selector.virtual_299",
    "selector.blitz_118",
    "selector.army_group_style_39_and_378",
    "resolved_definition_name",
    "route.builder_input_points_fixed5",
    "route.engine_pre_smoother_points_fixed5",
    "route.point_width_classes",
    "route.explicit_height_f32_bits",
    "route.wrap_offset_world",
    "manager_insertion_ordinal",
    "definition_group_ordinal",
    "object_width_multiplier_f32_bits",
    "comparator_tie_17c_f32_bits",
    "fade_in_f32_bits",
    "fade_out_f32_bits",
    "culling_offset_f32_bits",
    "inner_rgba_f32_bits",
    "outer_rgba_f32_bits",
    "texture_variant",
    "see_through",
    "visible",
    "final_engine_draw_ordinal",
    "exact_head_vertices_f32_bits",
    "lifecycle_tombstone_sha256",
];

/// EVERY movement-row leaf. Not one is sourced by a recorded domain.
///
/// `cross_check_d54_and_units` touches movement rows once, to require
/// `key.division_id` and `key.owner_country` to name exactly one division in
/// the XMAP13 identity census. That says the named division exists; nothing
/// says WHICH divisions owe a movement row, so the census itself is producer
/// chosen. `key.movement_generation` is not checked at all, and no D54 order
/// corresponds to a movement row.
pub const MOVEMENT_LEAVES_UNSOURCED: [&str; 29] = [
    "key.owner_country",
    "key.division_id",
    "key.movement_generation",
    "definition",
    "progress_raw_u32",
    "progress_move_army_f32_bits",
    "desaturation_lengths_f32_bits",
    "offset_x_f32_bits",
    "route.ordered_province_ids",
    "manager_insertion_ordinal",
    "definition_group_ordinal",
    "object_width_multiplier_f32_bits",
    "comparator_tie_17c_f32_bits",
    "fade_in_f32_bits",
    "fade_out_f32_bits",
    "inner_rgba_f32_bits",
    "outer_rgba_f32_bits",
    "texture_variant",
    "see_through",
    "visible",
    "final_engine_draw_ordinal",
    "exact_vertex_stride_bytes",
    "exact_vertex_count",
    "exact_vertex_bytes",
    "exact_index_format_u32",
    "exact_index_count",
    "exact_index_bytes",
    "exact_primitive_topology_u32",
    "lifecycle_tombstone_sha256",
];

/// The subset of [`MOVEMENT_LEAVES_UNSOURCED`] the viewer uploads and draws
/// verbatim, which is why no plausible substitute exists for them.
pub const MOVEMENT_FIELDS_SUBMITTED_MESH: [&str; 7] = [
    "exact_vertex_stride_bytes",
    "exact_vertex_count",
    "exact_vertex_bytes",
    "exact_index_format_u32",
    "exact_index_count",
    "exact_index_bytes",
    "exact_primitive_topology_u32",
];

pub const PENDING: PendingProducerCrate = PendingProducerCrate {
    crate_name: "xmap15_orders_semantic",
    ledger_row: "XMAP00 orders.army_battleplans_semantic",
    blocked_on:
        "all 29 movement-row leaves and 27 of the 38 plan-row leaves, counted in \
MOVEMENT_LEAVES_UNSOURCED and PLAN_LEAVES_UNSOURCED - leaves rather than fields because \
only route.ordered_province_ids of the plan route is pinned and the builder's five \
remaining route leaves are not. Seven of the movement leaves are \
the engine's SUBMITTED MESH rather than a description of it - exact_vertex_stride_bytes, \
exact_vertex_count, exact_vertex_bytes, exact_index_format_u32, exact_index_count, \
exact_index_bytes and exact_primitive_topology_u32 - because \
map_viewer/src/xmap15_d3d_handoff.rs uploads the vertex and index bytes into D3D buffers \
and then issues DrawIndexed(exact_index_count) for an indexed row or \
Draw(exact_vertex_count) for a non-indexed one, under exact_primitive_topology_u32. The \
only observer of a submitted mesh is domain 77's render session, which \
compact_contract::EARLY_LAUNCH_ONLY_PRODUCERS records as held off an attach capture by \
three independent gates. The other twenty-two movement leaves are ordinary arrow-object \
runtime state - the movement identity and generation, the definition and texture \
selection, the comparator and ordinal draw order, the colours, fades and visibility, the \
movement progress and desaturation lengths, the movement route and the lifecycle \
tombstone - and no recorded domain carries any of them either. What is NOT owed is the \
eleven plan-row leaves in PLAN_LEAVES_PINNED_BY_RECORDED_STATE, which \
map_viewer/src/production_features/xmap15.rs::cross_check_d54_and_units pins to domain \
54's recorded orders and the XMAP13 formation census. A second blocker sits behind all of it: prepare refuses \
without context.xmap15_units, the XMAP13 identity provider, whose own carrier \
(map_unit_presentation) is unfed on every recording",
    reason:
        "Xmap15SemanticFrame::validate refuses anything but a complete zero-omission \
current-hour census, so there is no partial shape to land: a producer written today would \
have to emit bytes for a mesh it never observed, and those bytes are uploaded and drawn \
verbatim. Omitting the key is what the consumer's own absent arm is written for",
};

pub const OWED_CARRIERS: [PendingProducerSlot; 1] = [PendingProducerSlot {
    wire_key: "xmap15_orders_fronts_movement_semantic",
    scope: "order_of_battle.extra",
    ledger_row: "XMAP00 orders.army_battleplans_semantic",
    consumer_citation: "replay_core/src/world.rs::xmap15_orders_fronts_movement_semantic",
    required_schema: RequiredSchema::Token("hoi4.xmap15-orders-fronts-movement-semantic.v1"),
    wire_absence: WireAbsence::OmitKey,
    reason: "the consumer's absent arm (production_features/xmap15.rs) types the family \
             Unavailable and names this exact key, so omitting it says so; a JSON null is \
             instead returned as Some(Value::Null) by the walk and fails \
             Xmap15SemanticFrame::decode_json, which reports a malformed carrier rather than a \
             clean absence",
}];

pub fn install() -> Result<(), String> {
    Err(PENDING.refusal())
}

#[cfg(test)]
mod tests {
    use super::*;
    use portable_render_stream::{
        Xmap15MovementDefinition, Xmap15MovementKey, Xmap15MovementRoute,
        Xmap15MovementSemanticRow, Xmap15OwnerKind, Xmap15PlanDefinitionSelector, Xmap15PlanKey,
        Xmap15PlanSemanticRow, Xmap15PreparedRoute,
    };
    use std::collections::BTreeSet;

    /// Every LEAF path of a serialized row, expanding nested objects.
    ///
    /// A field-level key set cannot express "only one part of this composite is
    /// pinned", which is how the whole `route` field was once classified as
    /// sourced when only its province vector is. Arrays and scalars are leaves;
    /// an `Option` that is `None` serializes to null and is a leaf, which is why
    /// the row builders below fill the optional composites' SIBLINGS rather than
    /// relying on any of them being `Some`.
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
                engine_pre_smoother_points_fixed5: None,
                point_width_classes: vec![0, 0],
                explicit_height_f32_bits: None,
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
            exact_head_vertices_f32_bits: None,
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

    /// kills: the declaration naming a SUBSET of what is unsourced -- the defect
    /// the first review round found, where seven geometry fields stood for the
    /// whole gap and twenty further movement fields went uncounted. A field
    /// added to either row type now fails here until it is classified, and the
    /// literal row constructors above fail to compile at the same time.
    #[test]
    fn every_mandatory_row_field_is_classified_exactly_once() {
        let plan = leaf_paths(&serde_json::to_value(plan_row()).expect("plan row serializes"));
        let movement =
            leaf_paths(&serde_json::to_value(movement_row()).expect("movement row serializes"));

        let mut classified_plan = BTreeSet::new();
        for name in PLAN_LEAVES_PINNED_BY_RECORDED_STATE
            .iter()
            .chain(PLAN_LEAVES_UNSOURCED.iter())
        {
            assert!(
                classified_plan.insert((*name).to_owned()),
                "plan leaf {name} is classified twice"
            );
        }
        assert_eq!(
            classified_plan, plan,
            "the plan-row classification and the schema disagree"
        );

        let mut classified_movement = BTreeSet::new();
        for name in MOVEMENT_LEAVES_UNSOURCED {
            assert!(
                classified_movement.insert(name.to_owned()),
                "movement leaf {name} is classified twice"
            );
        }
        assert_eq!(
            classified_movement, movement,
            "the movement-row classification and the schema disagree"
        );

        for name in MOVEMENT_FIELDS_SUBMITTED_MESH {
            assert!(
                MOVEMENT_LEAVES_UNSOURCED.contains(&name),
                "{name} is a submitted-mesh field and must also be counted as unsourced"
            );
        }
        // The composites are the reason this test walks leaves at all: if any
        // of them ever serialized as a scalar the walk would stop expanding and
        // the classification would silently shrink with it.
        for composite in ["key.owner_id", "selector.order_type", "route.wrap_offset_world"] {
            assert!(plan.contains(composite), "plan lost the composite leaf {composite}");
        }
        assert!(movement.contains("key.movement_generation"));
        assert!(movement.contains("route.ordered_province_ids"));
    }

    /// kills: landing a producer for this carrier while the fields are still
    /// unsourced -- the refusal would stop naming them and the next lane would
    /// read the carrier as merely unwired.
    #[test]
    fn an_unbuilt_producer_refuses_installation_and_names_the_unsourced_fields() {
        let error = install().expect_err("a pending producer must not claim an install");
        assert!(error.contains("xmap15_orders_semantic"), "{error}");
        for field in MOVEMENT_FIELDS_SUBMITTED_MESH {
            assert!(
                error.contains(field),
                "the refusal must name the submitted-mesh field {field}: {error}"
            );
        }
        assert!(
            error.contains(&format!("{} movement-row leaves", MOVEMENT_LEAVES_UNSOURCED.len())),
            "the refusal must carry the counted movement total: {error}"
        );
        assert!(
            error.contains(&format!("{} of the", PLAN_LEAVES_UNSOURCED.len())),
            "the refusal must carry the counted plan total: {error}"
        );
        assert!(
            error.contains("domain 77") || error.contains("EARLY_LAUNCH_ONLY_PRODUCERS"),
            "the refusal must name the observer that is missing: {error}"
        );
        assert!(
            error.contains("map_unit_presentation"),
            "the refusal must name the second blocker, XMAP13's identity provider: {error}"
        );
    }

    /// kills: the declaration telling a reader the opposite of the absence
    /// spelling it declares. The consumer's walk returns `Some(Value::Null)` for
    /// a present null key, so a null reaches `decode_json` and is reported as a
    /// malformed carrier; only an omitted key takes the named-unavailable arm.
    #[test]
    fn the_declared_absence_and_every_statement_about_it_agree() {
        assert!(!OWED_CARRIERS[0].wire_absence.emits_key_when_unfed());
        let published = format!("{}\0{}", PENDING.refusal(), OWED_CARRIERS[0].refusal());
        assert!(
            !published.contains("typed null"),
            "an OmitKey carrier must not tell a reader that a typed null is its absence: \
             {published}"
        );
        assert!(
            OWED_CARRIERS[0].reason.contains("Omitting")
                || OWED_CARRIERS[0].reason.contains("omitting"),
            "the slot reason must state the absence it declares: {}",
            OWED_CARRIERS[0].reason
        );
    }

    /// kills: re-declaring the plan half as owed. Domain 54 records it and the
    /// consumer requires the sidecar to agree with domain 54 rather than to
    /// introduce anything, so a lane sent at the plan census is sent at work
    /// that is done.
    #[test]
    fn the_declaration_separates_the_recorded_half_from_the_unobserved_half() {
        assert_eq!(BLOCKING_DOMAIN_ID, 77);
        assert_eq!(SEMANTIC_SOURCE_DOMAIN_ID, 54);
        assert!(
            compact_contract::early_launch_only_producer(BLOCKING_DOMAIN_ID)
                .is_some_and(|producer| producer.gates.len() > 1),
            "this carrier's blocker is d77's recorded attach-capture gate set"
        );
        let d54 = compact_contract::assigned_descriptor_by_id(SEMANTIC_SOURCE_DOMAIN_ID)
            .expect("d54 is an assigned domain");
        assert!(
            d54.schedulable,
            "the plan half is only 'already recorded' while d54 stays schedulable"
        );
        assert!(PENDING.blocked_on.contains("cross_check_d54_and_units"));
        for name in PLAN_LEAVES_PINNED_BY_RECORDED_STATE {
            assert!(
                !PLAN_LEAVES_UNSOURCED.contains(&name),
                "{name} cannot be both pinned and unsourced"
            );
        }
    }

    #[test]
    fn owed_carriers_record_the_schema_their_consumer_requires() {
        assert_eq!(
            OWED_CARRIERS[0].required_schema.token(),
            Some("hoi4.xmap15-orders-fronts-movement-semantic.v1")
        );
        for carrier in &OWED_CARRIERS {
            carrier.validate().expect("owed carrier must be well formed");
        }
        PENDING
            .validate()
            .expect("the crate declaration must be well formed");
    }
}

