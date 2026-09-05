//! The XMAP15 orders/fronts/movement sidecar producer.
//!
//! `order_of_battle.extra.xmap15_orders_fronts_movement_semantic` is read every
//! hour by `map_viewer/src/production_features/xmap15.rs` and no extractor
//! writes it. Lane WL classified its two row types leaf by leaf; the
//! classification now lives in shared ground as
//! `portable_render_stream::XMAP15_PLAN_ROW_CONTRACT` and its two siblings,
//! where the crate that owns the SCHEMA is bound to it, and this crate carries
//! the producer and the gate instead of a second copy of the list.
//!
//! # What this producer does and does not emit
//!
//! It mints the eleven plan-row leaves the consumer pins to recorded state -
//! the order identity, its slot, its type, its active flag, its assigned
//! divisions, its province route and its owner's country - from d54's own
//! orders and the XMAP13 formation census. It emits NO carrier. The carrier's
//! declared absence is `WireAbsence::OmitKey`, and unlike XMAP13's typed null
//! that is not a stylistic choice: `Xmap15SemanticFrame::validate` requires
//! `omitted_movement_rows == 0` and `movement_object_census ==
//! movements.len()`, so a frame with an empty movement vector is the positive
//! claim that the engine had no movement arrow objects this hour - a
//! measurement no producer that has not observed them can make. What goes on
//! the wire instead is the per-hour gate beside the omitted key.

use compact_contract::pending_producer::{
    PendingProducerCrate, PendingProducerSlot, RequiredSchema, WireAbsence,
};
use portable_render_stream::{
    Xmap15HourGate, Xmap15ProducerGate, XMAP15_PRODUCER_GATE_SCHEMA,
};
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;

/// The producer-side domain whose capture shape decides this carrier.
pub const BLOCKING_DOMAIN_ID: u16 = 77;

/// The domain that carries the plan half the consumer cross-checks.
pub const SEMANTIC_SOURCE_DOMAIN_ID: u16 = 54;

pub const PENDING: PendingProducerCrate = PendingProducerCrate {
    crate_name: "xmap15_orders_semantic",
    ledger_row: "XMAP00 orders.army_battleplans_semantic",
    blocked_on:
        "every movement-row leaf and 20 of the 38 plan-row leaves remain unproven. \
         Existing D54 records exact plan identities, routes and style predicates; the viewer \
         derives definitions and province anchors independently. Those anchors do not prove \
         builder control points, widths, runtime appearance, lifecycle or complete movement \
         census. The full native carrier cannot honestly claim measured-empty movement rows",
    reason:
        "project_fold_sections publishes the existing per-hour gate from admitted D54 and \
         formation inputs. There is no scheduled producer for the full native carrier; \
         deriving plan inputs does not establish its geometry or movement census",

};

pub const OWED_CARRIERS: [PendingProducerSlot; 1] = [PendingProducerSlot {
    wire_key: portable_render_stream::XMAP15_CARRIER_WIRE_PATH[2],
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

/// One XMAP13 formation, reduced to what a plan owner's country needs.
///
/// The producer takes these rather than depending on the XMAP13 producer crate:
/// the fold has both censuses in hand, and a crate direction between two lane
/// producers would make the second one's tests need the first one's fold input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormationOwner {
    pub is_army_group: bool,
    pub owner_id: u32,
    pub owner_country: String,
}

/// One owner in d54's manager identity census, without a country.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FormationIdentity {
    pub is_army_group: bool,
    pub owner_id: u32,
}

/// What XMAP13 got out of the orders-group manager this hour.
///
/// Three answers rather than one, because there are three states and collapsing
/// them names the wrong cause. The IDENTITY census and the COUNTRIES fail
/// independently: an unreadable manager means replay_core reports no d54 view
/// at all, while a manager it read whose owners have no country leaves the view
/// standing and refuses only XMAP13's own rows. And which recorded input a
/// country is missing from - the divisions or the theatre selector - is
/// XMAP13's answer, not a guess this producer may make.
pub struct Xmap13Formations<'a> {
    /// The manager identity census, or `None` where replay_core refuses the
    /// manager and with it every order row admitted against it.
    pub owner_census: Option<&'a [FormationIdentity]>,
    /// Those owners with their countries, whenever every one resolved.
    pub with_countries: &'a [FormationOwner],
    /// The recorded input XMAP13's own formation census is gated on.
    pub gate: Option<&'static str>,
}

/// One visual d54 order, reduced to the leaves this producer may mint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordedPlanOrder {
    pub owner_is_army_group: bool,
    pub owner_id: u32,
    pub engine_order_instance_id: u32,
    pub generation: u64,
    pub slot_index: u32,
    pub order_type: u8,
    pub active: bool,
    pub assigned: bool,
    pub ordered_assigned_division_ids: Vec<u32>,
    pub route_province_ids: Vec<u32>,
    pub owner_country: String,
}

/// One hour of recorded state, as the fold has it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RecordedHour {
    pub hour_index: i64,
    pub plans: Vec<RecordedPlanOrder>,
    /// The visual d54 orders this hour carried, whether or not each minted.
    ///
    /// Reported beside the mint because `cross_check_d54_and_units` requires
    /// the two to be EQUAL, so a mint short of this count is a different fact
    /// from an hour with no visual orders at all.
    pub visual_d54_orders: u32,
    pub hour_gates: Vec<Xmap15HourGate>,
}

impl RecordedHour {
    pub fn gate(&self) -> Xmap15ProducerGate {
        Xmap15ProducerGate {
            schema: XMAP15_PRODUCER_GATE_SCHEMA.to_owned(),
            hour_index: self.hour_index,
            minted_plan_rows: u32::try_from(self.plans.len()).unwrap_or(u32::MAX),
            visual_d54_orders: self.visual_d54_orders,
            hour_gates: self.hour_gates.clone(),
        }
    }
}

/// What the producer got out of one hour.
#[derive(Clone, Debug, PartialEq)]
pub struct Production {
    /// One object per visual d54 order, carrying exactly the leaves the
    /// contract classifies as this producer's.
    pub minted_plan_rows: Vec<Value>,
    pub gate: Xmap15ProducerGate,
}

/// Mint the recorded half of this hour's plan census.
pub fn produce(hour: &RecordedHour) -> Production {
    let minted_plan_rows = hour
        .plans
        .iter()
        .map(|plan| {
            let mut row = Map::new();
            row.insert(
                "key".into(),
                json!({
                    "owner_kind": if plan.owner_is_army_group { "army_group" } else { "army" },
                    "owner_id": plan.owner_id,
                    "engine_order_instance_id": plan.engine_order_instance_id,
                    // The schema serializes this u64 as decimal TEXT, which is
                    // how a generation past 2^53 survives a JSON reader.
                    "generation": plan.generation.to_string(),
                    "slot_index": plan.slot_index,
                }),
            );
            row.insert(
                "owner_country".into(),
                Value::String(plan.owner_country.clone()),
            );
            row.insert("selector".into(), json!({ "order_type": plan.order_type }));
            row.insert(
                "ordered_assigned_division_ids".into(),
                json!(plan.ordered_assigned_division_ids),
            );
            row.insert("assigned".into(), Value::Bool(plan.assigned));
            row.insert("active".into(), Value::Bool(plan.active));
            row.insert(
                "route".into(),
                json!({ "ordered_province_ids": plan.route_province_ids }),
            );
            Value::Object(row)
        })
        .collect();
    Production {
        minted_plan_rows,
        gate: hour.gate(),
    }
}

/// Project one folded hour's own published sections into this producer's input.
///
/// Reads d54 through the same section and the same field names
/// `replay_core/src/army_battleplans.rs` builds its typed view from, and is at
/// least as strict everywhere the strictness decides which orders are in the
/// census: the consumer requires this producer's plan count to EQUAL the visual
/// d54 count, so an order admitted here and refused there, or the reverse, is a
/// census mismatch rather than a row-level disagreement.
pub fn project_fold_sections(
    hour_index: i64,
    order_of_battle: &Value,
    formations: &Xmap13Formations<'_>,
) -> RecordedHour {
    let mut hour = RecordedHour {
        hour_index,
        ..RecordedHour::default()
    };
    let domain = order_of_battle
        .get("extra")
        .and_then(|extra| extra.get("army_battleplans_depth_live"))
        .filter(|domain| domain.get("known") == Some(&Value::Bool(true)))
        .and_then(Value::as_object);
    let Some(domain) = domain else {
        return gate(
            &mut hour,
            D54,
            "the recording carries no known army_battleplans_depth_live section this hour",
        );
    };
    // The consumer refuses anything but a complete zero-omission view, so an
    // hour that omitted a row has no census this producer can complete.
    if domain.get("baseline_seen") != Some(&Value::Bool(true))
        || ["omitted_non_exact_rows_this_hour",
            "path_geometry_omitted_rows_this_hour",
            "identity_or_active_omitted_rows_this_hour"]
            .iter()
            .any(|key| domain.get(*key).and_then(Value::as_u64) != Some(0))
    {
        return gate(
            &mut hour,
            D54,
            "the d54 view is not a complete zero-omission baseline, and \
             xmap15.rs::cross_check_d54_and_units refuses anything less",
        );
    }
    let Some(orders) = domain.get("orders").and_then(Value::as_array) else {
        return gate(&mut hour, D54, "the d54 orders section is not an array");
    };
    // TWO PASSES, and the first one is the whole hour's census.
    //
    // What `parse_order` REFUSES is not a dropped row. replay_core reports the
    // WHOLE view unavailable for it, and the consumer then refuses at
    // `d54.status != Present` before its visual filter ever runs - so a refused
    // row costs the hour, and an hour with no view has no visual census to
    // count rather than a short one. Only rows the view admits reach the second
    // pass, where the count is already complete however that pass ends.
    // Owner admission needs the manager's identity census, and replay_core
    // refuses the whole view without one - it admits every order row against
    // that census, so an hour whose manager it will not read is an hour with no
    // battleplan view whatever the orders section holds.
    let Some(owner_census) = formations.owner_census else {
        return view_unavailable(
            &mut hour,
            "replay_core refuses the orders-group manager this hour, and it admits every d54 \
             order row against that manager's identity census",
        );
    };
    let mut visual = Vec::new();
    let mut seen_identities: BTreeSet<(bool, u32, u32, u64)> = BTreeSet::new();
    for order in orders {
        let Some(order) = order.as_object() else {
            return view_unavailable(
                &mut hour,
                "a d54 order row is not an object, and replay_core reports the whole view \
                 unavailable for it rather than dropping the row",
            );
        };
        let D54Order::Admitted(admitted) = d54_order(order) else {
            return view_unavailable(
                &mut hour,
                "a d54 order row violates the exact carrier contract, and replay_core reports \
                 the whole view unavailable for it rather than dropping the row",
            );
        };
        if !seen_identities.insert((
            admitted.owner_is_army_group,
            admitted.owner_id,
            admitted.engine_order_instance_id,
            admitted.generation,
        )) {
            return view_unavailable(
                &mut hour,
                "two d54 order rows carry one stable identity, and replay_core reports the whole \
                 view unavailable for that",
            );
        }
        // Owner admission, over EVERY admitted row rather than the visual
        // ones: replay_core refuses the view for an owner outside the manager's
        // identity census whatever that order would have drawn. This is asked
        // of the IDENTITY census, which stands even where the countries do not
        // - the two fail independently, and asking it of the countries let an
        // orphan owner through on every hour whose countries were gated.
        if !owner_census.iter().any(|owner| {
            owner.is_army_group == admitted.owner_is_army_group
                && owner.owner_id == admitted.owner_id
        }) {
            return view_unavailable(
                &mut hour,
                "a d54 order's owner is absent from the orders-group manager identity census, \
                 and replay_core reports the whole view unavailable for it",
            );
        }
        let country = formations
            .with_countries
            .iter()
            .find(|formation| {
                formation.is_army_group == admitted.owner_is_army_group
                    && formation.owner_id == admitted.owner_id
            })
            .map(|formation| formation.owner_country.clone());
        // Every admitted row's owner kind is Army or ArmyGroup, so what is left
        // of the consumer's visual filter is the order type.
        if !(1..=4).contains(&admitted.order_type) {
            continue;
        }
        hour.visual_d54_orders = hour.visual_d54_orders.saturating_add(1);
        visual.push(admitted.mintable.map(|parsed| RecordedPlanOrder {
            owner_country: country.unwrap_or_default(),
            ..parsed
        }));
    }
    // No formation census, no plan census - whatever the d54 view holds. The
    // consumer resolves every plan owner through XMAP13 and has no identity
    // provider at all for an hour XMAP13 gated, so this returns BEFORE any row
    // is minted, which is also what leaves the blank country above unreachable.
    // The input is the one XMAP13 named rather than a guess: the two producers
    // must not disagree about which recorded input this hour is missing.
    if let Some(missing_input) = formations.gate {
        return gate(
            &mut hour,
            missing_input,
            "the XMAP13 formation census is unavailable this hour, and \
             xmap15.rs::cross_check_d54_and_units resolves every plan owner through it",
        );
    }
    let mut plans = Vec::with_capacity(visual.len());
    for parsed in visual {
        let Some(parsed) = parsed else {
            // A visual order whose geometry d54 could not measure. It is IN the
            // census - replay_core admits the row and the consumer's filter
            // keeps it - and `cross_check_d54_and_units` then refuses the whole
            // frame at exactly that, so no plan census this producer could
            // build would be admitted this hour. The VIEW stands, so the visual
            // count stands with it.
            return gate(
                &mut hour,
                D54,
                "a visual d54 order carries no exact path geometry, and \
                 xmap15.rs::cross_check_d54_and_units refuses the frame at exactly that",
            );
        };
        plans.push(parsed);
    }
    hour.plans = plans;
    hour
}

const D54: &str = "orders.army_battleplans_semantic";
/// The two inputs an XMAP13 formation census can be missing, named here only so
/// the tests can state which one this producer must pass through untouched.
///
/// This producer no longer CHOOSES between them: an army's country comes from
/// its divisions and an army group's from the theatre selector, and XMAP13 is
/// the reader that knows which of the two failed. Picking one here made half
/// the gated hours name a section that was not missing anything.
#[cfg(test)]
const HIERARCHY: &str = "orders.theatre_army_hierarchy";
#[cfg(test)]
const DIVISIONS: &str = "units.divisions";

/// Gate the hour on d54 AND withdraw the visual census with it.
///
/// The count is a measurement of what the VIEW held, and replay_core reports no
/// view at all for any of these - so there is no census to have counted, and a
/// number here would be a claim about a view that does not exist. Leaving the
/// running total in place also made the number depend on where in the hour the
/// refused row sat, which is a whole-hour census that changes with row order.
///
/// This is exactly not what happens to a view that STANDS while a row of it
/// cannot be minted: there the count is the real census and survives the gate.
fn view_unavailable(hour: &mut RecordedHour, recorded_state: &str) -> RecordedHour {
    hour.visual_d54_orders = 0;
    gate(hour, D54, recorded_state)
}

/// Record one hour gate, at most once per recorded input, and return the hour.
///
/// The first reason is kept: `Xmap15ProducerGate::validate` refuses a gate that
/// names one input twice, and a duplicate would make the whole gate unusable at
/// the consumer rather than merely read oddly.
fn gate(
    hour: &mut RecordedHour,
    recorded_input: &str,
    recorded_state: &str,
) -> RecordedHour {
    if !hour
        .hour_gates
        .iter()
        .any(|existing| existing.recorded_input == recorded_input)
    {
        hour.hour_gates.push(Xmap15HourGate {
            recorded_input: recorded_input.to_owned(),
            recorded_state: recorded_state.to_owned(),
        });
    }
    // Every census here is all-or-nothing, so a gated hour mints nothing at
    // all: a partial plan census is a count mismatch at the consumer, which
    // names the census rather than the input that was missing.
    //
    // The VISUAL COUNT survives, because it is a measurement of what d54
    // carried and not a claim about what was minted. Zeroing it here made the
    // gate say the hour had no visual orders when it had some it could not
    // mint, which are the two facts the gate exists to keep apart.
    RecordedHour {
        hour_index: hour.hour_index,
        plans: Vec::new(),
        visual_d54_orders: hour.visual_d54_orders,
        hour_gates: std::mem::take(&mut hour.hour_gates),
    }
}

/// The exact field set `replay_core::army_battleplans::parse_order` admits.
///
/// It admits a row of EXACTLY these plus `owner_kind` and `owner_id`, and
/// refuses any other field count. That refusal is part of the census: an order
/// with an extra key is not in the view this producer's census must equal, so
/// admitting it here would put a plan in one census and not the other.
const D54_BASE_FIELDS: [&str; 19] = portable_render_stream::XMAP15_D54_ORDER_ROW_FIELDS;

/// The status words the row SHAPE pins, and the value each is pinned to.
///
/// `target_status` is deliberately not among them: parse_order pins it per
/// order type - "exact" at type 5, "unsupported" everywhere else - so a flat
/// entry here would refuse a row the view admits.
const D54_PINNED_STATUS: [(&str, &str); 4] = [
    ("execution_status", "exact"),
    ("assigned_units_status", "exact"),
    ("order_linkage_status", "exact"),
    ("active_status", "exact"),
];

/// The engine ranges parse pins, taken from the ONE spelling of them.
///
/// They were spelled again here once, and the order instance bound was wrong:
/// this producer allowed ids to i32::MAX where the consumer stops at a hundred
/// million, so a row above that was minted here and made the whole view
/// unavailable there. Both sides now read portable_render_stream.
const D54_MAX_ARMY_ID: u32 = portable_render_stream::XMAP15_D54_MAX_ARMY_ID;
const D54_MAX_ORDER_INSTANCE_ID: u32 = portable_render_stream::XMAP15_D54_MAX_ORDER_INSTANCE_ID;
const D54_MAX_MAP_ID: u32 = portable_render_stream::XMAP15_D54_MAX_MAP_ID;
const D54_SLOT_INDEX_COUNT: u32 = portable_render_stream::XMAP15_D54_SLOT_INDEX_COUNT;

/// One order the d54 view ADMITS, and whether XMAP15 can mint it.
pub(crate) struct AdmittedOrder {
    owner_is_army_group: bool,
    owner_id: u32,
    engine_order_instance_id: u32,
    generation: u64,
    order_type: i32,
    /// `None` where the view admitted the row with geometry it could not
    /// measure, or with geometry outside the visual family. Such a row is in
    /// the view and cannot be minted, which is a third answer and not a refusal.
    mintable: Option<RecordedPlanOrder>,
}

/// Whether the d54 VIEW admits a row at all.
///
/// `replay_core::army_battleplans::parse_order` returning `None` does not drop
/// the row: replay_core reports the whole view unavailable, and the consumer
/// refuses at `d54.status != Present` before its visual filter runs. So the two
/// answers here are "the view stands, and here is the row" and "the view is
/// gone" - reading a refused row as merely outside this family let the producer
/// publish a plan census for an hour the consumer shows nothing of.
///
/// Reproducing the admission field for field is the point: the consumer
/// requires this producer's plan count to EQUAL the visual d54 count, so a row
/// admitted on one side and refused on the other is a census mismatch that
/// names neither the row nor the field that differed. That includes the order
/// types this family never draws - a malformed type 5 costs the view too.
///
/// `owner_country` is left blank on a minted row and filled by the caller from
/// the formation census.
pub(crate) enum D54Order {
    Refused,
    Admitted(AdmittedOrder),
}

fn d54_order(order: &Map<String, Value>) -> D54Order {
    // The exact field set, including the count. A row carrying an extra key is
    // refused before any value is read.
    if !D54_BASE_FIELDS.iter().all(|field| order.contains_key(*field)) {
        return D54Order::Refused;
    }
    let typed = order.len() == D54_BASE_FIELDS.len() + 2
        && order.contains_key("owner_kind")
        && order.contains_key("owner_id");
    if !typed {
        return D54Order::Refused;
    }
    if order.get("linked_orders").and_then(portable_render_stream::parse_d54_order_links).is_none() {
        return D54Order::Refused;
    }
    for (key, expected) in D54_PINNED_STATUS {
        if order.get(key).and_then(Value::as_str) != Some(expected) {
            return D54Order::Refused;
        }
    }
    let (
        Some(identity),
        Some(generation),
        Some(active),
        Some(order_type),
        Some(target_states),
        Some(path_provinces),
        Some(ordered_assigned_division_ids),
    ) = (
        order_identity(order),
        order.get("generation").and_then(Value::as_u64),
        order.get("active").and_then(Value::as_bool),
        order
            .get("order_type")
            .and_then(Value::as_i64)
            .and_then(|value| i32::try_from(value).ok()),
        order
            .get("target_states")
            .and_then(|value| strict_id_array(value, D54_MAX_MAP_ID)),
        order
            .get("path_provinces")
            .and_then(|value| strict_id_array(value, D54_MAX_MAP_ID)),
        order
            .get("assigned_division_ids")
            .and_then(|value| strict_id_array(value, D54_MAX_ARMY_ID)),
    )
    else {
        return D54Order::Refused;
    };
    // Keep producer admission identical to replay_core::army_battleplans::parse_style.
    let Some(style_flags) = u32_of(order, "style_flags") else {
        return D54Order::Refused;
    };
    match order.get("style_status").and_then(Value::as_str) {
        Some("exact")
            if style_flags & !portable_render_stream::XMAP15_D54_STYLE_FLAG_MASK == 0 => {}
        Some("validation_failed" | "unknown" | "overflow" | "unsupported" | "fixture_required"
            | "cold_preflight_only") if style_flags == 0 => {}
        _ => return D54Order::Refused,
    }
    let target_declared_count_is_null = order
        .get("target_declared_count")
        .is_some_and(Value::is_null);
    let target_status = order.get("target_status").and_then(Value::as_str);
    let geometry_status = order.get("geometry_status").and_then(Value::as_str);
    // Geometry, exactly as parse_order decides it per order type. `Some` is a
    // row this producer can mint; `None` is a row the view holds and this
    // producer cannot.
    let route: Option<Vec<u32>> = match order_type {
        1..=4
            if !target_states.is_empty()
                || !target_declared_count_is_null
                || target_status != Some("unsupported") =>
        {
            return D54Order::Refused
        }
        1..=3 => match (geometry_status, path_provinces.is_empty()) {
            (Some("exact"), false) => Some(path_provinces),
            (Some("fixture_required"), true) => None,
            _ => return D54Order::Refused,
        },
        4 if geometry_status == Some("exact") && path_provinces.len() == 2 => Some(path_provinces),
        4 => return D54Order::Refused,
        5 => {
            if !path_provinces.is_empty()
                || order
                    .get("target_declared_count")
                    .and_then(Value::as_u64)
                    .and_then(|value| usize::try_from(value).ok())
                    != Some(target_states.len())
                || target_status != Some("exact")
                || geometry_status != Some("exact")
            {
                return D54Order::Refused;
            }
            None
        }
        _ => {
            if !target_states.is_empty()
                || !path_provinces.is_empty()
                || !target_declared_count_is_null
                || target_status != Some("unsupported")
                || geometry_status != Some("fixture_required")
            {
                return D54Order::Refused;
            }
            None
        }
    };
    let mintable = route.and_then(|route_province_ids| {
        let order_type = u8::try_from(order_type).ok()?;
        Some(RecordedPlanOrder {
            owner_is_army_group: identity.owner_is_army_group,
            owner_id: identity.owner_id,
            engine_order_instance_id: identity.engine_order_instance_id,
            generation,
            slot_index: identity.slot_index,
            order_type,
            active,
            // `Xmap15SemanticFrame::validate` ties `assigned` to whether the
            // division vector is empty, so it has no independent value.
            assigned: !ordered_assigned_division_ids.is_empty(),
            ordered_assigned_division_ids,
            route_province_ids,
            owner_country: String::new(),
        })
    });
    D54Order::Admitted(AdmittedOrder {
        owner_is_army_group: identity.owner_is_army_group,
        owner_id: identity.owner_id,
        engine_order_instance_id: identity.engine_order_instance_id,
        generation,
        order_type,
        mintable,
    })
}

struct OrderIdentity {
    owner_is_army_group: bool,
    owner_id: u32,
    engine_order_instance_id: u32,
    slot_index: u32,
}

/// The identity half of the admission, including its engine ranges.
fn order_identity(order: &Map<String, Value>) -> Option<OrderIdentity> {
    let (owner_is_army_group, owner_id) = {
        let owner_id = u32_of(order, "owner_id")?;
        match order.get("owner_kind")?.as_str()? {
            "army" if u32_of(order, "army_id") == Some(owner_id) => (false, owner_id),
            "army_group" if order.get("army_id")?.is_null() => (true, owner_id),
            _ => return None,
        }
    };
    let engine_order_instance_id = u32_of(order, "engine_order_instance_id")?;
    let slot_index = u32::try_from(order.get("slot_index")?.as_u64()?).ok()?;
    (owner_id != 0
        && owner_id <= D54_MAX_ARMY_ID
        && engine_order_instance_id != 0
        && engine_order_instance_id <= D54_MAX_ORDER_INSTANCE_ID
        && slot_index < D54_SLOT_INDEX_COUNT)
        .then_some(OrderIdentity {
            owner_is_army_group,
            owner_id,
            engine_order_instance_id,
            slot_index,
        })
}

/// An id array parse_order would accept: every entry a nonzero u32 within the
/// given bound, and no duplicates.
fn strict_id_array(value: &Value, max: u32) -> Option<Vec<u32>> {
    let mut seen = std::collections::BTreeSet::new();
    let ids: Vec<u32> = value
        .as_array()?
        .iter()
        .map(|entry| {
            let id = u32::try_from(entry.as_u64()?).ok()?;
            (id != 0 && id <= max && seen.insert(id)).then_some(id)
        })
        .collect::<Option<_>>()?;
    Some(ids)
}

fn u32_of(object: &Map<String, Value>, key: &str) -> Option<u32> {
    u32::try_from(object.get(key)?.as_u64()?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use portable_render_stream::{
        xmap15_contract_gate_leaves, xmap15_minted_plan_leaves,
        validate_xmap15_producer_contract, XMAP15_PLAN_ROW_CONTRACT,
    };
    use std::collections::BTreeSet;

    #[test]
    fn an_unbuilt_carrier_refuses_installation_and_names_its_counted_gates() {
        let error = install().expect_err("a pending producer must not claim an install");
        assert_eq!(error, PENDING.refusal());
        assert!(!OWED_CARRIERS[0].wire_absence.emits_key_when_unfed());
        let plan_gates = XMAP15_PLAN_ROW_CONTRACT
            .iter()
            .filter(|(_, source)| source.gate_owner().is_some())
            .count();
        assert!(
            PENDING
                .blocked_on
                .contains(&format!("{plan_gates} of the 38 plan-row leaves")),
            "the refusal must carry the contract's own plan count ({plan_gates}): {}",
            PENDING.blocked_on
        );
        validate_xmap15_producer_contract().expect("the shared contract must be well formed");
        PENDING.validate().expect("the declaration must be well formed");
        assert_eq!(BLOCKING_DOMAIN_ID, 77);
        assert_eq!(SEMANTIC_SOURCE_DOMAIN_ID, 54);
    }

    #[test]
    fn the_declared_absence_omits_the_key_and_every_statement_about_it_agrees() {
        assert!(!OWED_CARRIERS[0].wire_absence.emits_key_when_unfed());
        OWED_CARRIERS[0].validate().expect("the slot must be well formed");
        // The key and its scope are the SHARED path, not a second spelling. A
        // rename on the shared side has to reach this declaration, or the
        // producer's refusal would name a key nothing reads.
        let path = portable_render_stream::XMAP15_CARRIER_WIRE_PATH;
        assert_eq!(OWED_CARRIERS[0].wire_key, path[2]);
        assert_eq!(OWED_CARRIERS[0].scope, format!("{}.{}", path[0], path[1]));
        let published = format!("{}\0{}", PENDING.refusal(), OWED_CARRIERS[0].refusal());
        assert!(
            !published.contains("typed null"),
            "an OmitKey carrier must not tell a reader that a typed null is its absence: \
             {published}"
        );
        assert!(!xmap15_contract_gate_leaves().is_empty());
    }

    #[test]
    fn style_admission_matches_reader_without_defaulting_unread_predicates() {
        let statuses = [
            "exact", "validation_failed", "unknown", "overflow", "unsupported",
            "fixture_required", "cold_preflight_only", "unrecognized",
        ];
        for status in statuses {
            for flags in 0..=128u32 {
                let (mut folded, formations) = one_folded_hour();
                let row = &mut folded["extra"]["army_battleplans_depth_live"]["orders"][0];
                row["style_flags"] = json!(flags);
                row["style_status"] = json!(status);
                let admitted = match status {
                    "exact" => flags <= portable_render_stream::XMAP15_D54_STYLE_FLAG_MASK,
                    "unrecognized" => false,
                    _ => flags == 0,
                };
                let hour = project(&folded, &formations, &formations, None);
                assert_eq!(hour.plans.len(), usize::from(admitted), "{status} {flags}");
                assert_eq!(hour.hour_gates.is_empty(), admitted, "{status} {flags}");
            }
        }
        for bad_flags in [Value::Null, json!(-1), json!(1u64 << 32), json!("0"), json!(0.5)] {
            let (mut folded, formations) = one_folded_hour();
            folded["extra"]["army_battleplans_depth_live"]["orders"][0]["style_flags"] = bad_flags;
            let hour = project(&folded, &formations, &formations, None);
            assert!(hour.plans.is_empty());
            assert!(!hour.hour_gates.is_empty());
        }
    }

    /// The handoff for an hour whose MANAGER census stands.
    ///
    /// `owners` is that census - the owners replay_core admits an order row
    /// against - and `formations` is however much of it has a country, which is
    /// the whole census unless `gate` says which input the rest is missing from.
    /// An hour whose manager replay_core refuses is a different shape and the
    /// tests that mean it construct it directly.
    fn project(
        order_of_battle: &Value,
        owners: &[FormationOwner],
        formations: &[FormationOwner],
        gate: Option<&'static str>,
    ) -> RecordedHour {
        let owner_census: Vec<FormationIdentity> = owners
            .iter()
            .map(|owner| FormationIdentity {
                is_army_group: owner.is_army_group,
                owner_id: owner.owner_id,
            })
            .collect();
        project_fold_sections(
            17,
            order_of_battle,
            &Xmap13Formations {
                owner_census: Some(&owner_census),
                with_countries: formations,
                gate,
            },
        )
    }

    fn one_folded_hour() -> (Value, Vec<FormationOwner>) {
        let order = json!({
            "army_id": 5,
            "owner_kind": "army",
            "owner_id": 5,
            "engine_order_instance_id": 2025,
            "generation": 4,
            "slot_index": 1,
            "order_type": 2,
            "target_states": [],
            "path_provinces": [101, 102],
            "target_declared_count": null,
            "target_status": "unsupported",
            "geometry_status": "exact",
            "execution_status": "exact",
            "assigned_division_ids": [17],
            "assigned_units_status": "exact",
            "order_linkage_status": "exact",
            "active": true,
            "active_status": "exact",
            "style_flags": 0,
            "style_status": "exact",
            "linked_orders": {"version": 3, "status": 2, "predecessors": [], "successors": [], "virtual_predecessors": [], "virtual_successors": []},
        });
        let order_of_battle = json!({
            "extra": {
                "army_battleplans_depth_live": {
                    "known": true,
                    "baseline_seen": true,
                    "omitted_non_exact_rows_this_hour": 0,
                    "path_geometry_omitted_rows_this_hour": 0,
                    "identity_or_active_omitted_rows_this_hour": 0,
                    "provenance_notes": [],
                    "orders": [order],
                },
            },
        });
        let formations = vec![FormationOwner {
            is_army_group: false,
            owner_id: 5,
            owner_country: "GER".into(),
        }];
        (order_of_battle, formations)
    }

    /// kills: the mint drifting off the contract in either direction.
    ///
    /// The eleven pinned leaves are what the consumer compares against d54 and
    /// the XMAP13 census. One more would be a second writer on a fact the
    /// viewer derives; one fewer would emit a row refused for a reason no gate
    /// names.
    #[test]
    fn a_minted_plan_row_carries_exactly_the_contracts_recorded_leaves() {
        let (order_of_battle, formations) = one_folded_hour();
        let hour = project(&order_of_battle, &formations, &formations, None);
        assert!(hour.hour_gates.is_empty(), "{:?}", hour.hour_gates);
        assert_eq!(hour.visual_d54_orders, 1);
        let production = produce(&hour);
        production.gate.validate().expect("a well-formed gate");
        assert_eq!(production.minted_plan_rows.len(), 1);

        fn leaves(value: &Value, prefix: &str, out: &mut BTreeSet<String>) {
            match value {
                Value::Object(fields) => {
                    for (name, child) in fields {
                        let path = if prefix.is_empty() {
                            name.clone()
                        } else {
                            format!("{prefix}.{name}")
                        };
                        leaves(child, &path, out);
                    }
                }
                _ => {
                    out.insert(prefix.to_owned());
                }
            }
        }
        let mut minted = BTreeSet::new();
        leaves(&production.minted_plan_rows[0], "", &mut minted);
        let wanted: BTreeSet<String> = xmap15_minted_plan_leaves()
            .into_iter()
            .map(str::to_owned)
            .collect();
        assert_eq!(minted, wanted);

        // The generation is decimal TEXT on the wire, which is how a value past
        // 2^53 survives a JSON reader that would otherwise round it.
        assert_eq!(
            production.minted_plan_rows[0]["key"]["generation"],
            json!("4")
        );
    }

    /// kills: this producer admitting an order `replay_core`'s own parse
    /// refuses.
    ///
    /// `cross_check_d54_and_units` requires the plan census to EQUAL the visual
    /// d54 count, so an order admitted on one side and refused on the other is
    /// a census mismatch that names neither the row nor the field that
    /// differed. The first version of this test picked five mutations by hand
    /// and passed while the producer ignored `target_states`,
    /// `target_declared_count`, `target_status`, the canonical field set, the
    /// engine id ranges and duplicate ids -- so the mutations are now DERIVED
    /// from the contract itself, and a pin added to that contract without a
    /// check in the producer fails here.
    #[test]
    fn an_order_the_replay_parse_refuses_is_not_in_this_census_either() {
        let mut mutations: Vec<(String, Box<dyn Fn(&mut Value)>)> = Vec::new();
        // Every pinned status word, broken one at a time.
        for (key, expected) in D54_PINNED_STATUS {
            let key = key.to_owned();
            let wrong = if expected == "exact" { "unknown" } else { "exact" };
            mutations.push((
                format!("{key} is not {expected}"),
                Box::new(move |order: &mut Value| order[&key] = json!(wrong)),
            ));
        }
        // Every field of the canonical set, removed one at a time. parse_order
        // refuses on the field COUNT before it reads a value.
        for field in D54_BASE_FIELDS {
            mutations.push((
                format!("{field} is absent"),
                Box::new(move |order: &mut Value| {
                    order.as_object_mut().expect("an order row").remove(field);
                }),
            ));
        }
        for field in ["virtual_predecessors", "virtual_successors"] {
            mutations.push((format!("{field} missing from version 3"), Box::new(move |order: &mut Value| {
                order["linked_orders"].as_object_mut().unwrap().remove(field);
            })));
        }
        for (label, mutate) in [
            (
                "an unsupported link grammar version",
                Box::new(|order: &mut Value| order["linked_orders"]["version"] = json!(2))
                    as Box<dyn Fn(&mut Value)>,
            ),
            (
                "an unknown link status",
                Box::new(|order: &mut Value| order["linked_orders"]["status"] = json!(3)),
            ),
            (
                "non-exact links carrying an identity",
                Box::new(|order: &mut Value| order["linked_orders"]["successors"] = json!([{
                    "owner_kind": 0, "owner_id": 17, "generation": 1, "engine_order_instance_id": 3
                }])),
            ),
            (
                "an extra field the canonical set does not carry",
                Box::new(|order: &mut Value| order["extra_key"] = json!(1))
                    as Box<dyn Fn(&mut Value)>,
            ),
            (
                "an owner_id that disagrees with army_id",
                Box::new(|order: &mut Value| order["owner_id"] = json!(6)),
            ),
            (
                "an army group whose army_id is not null",
                Box::new(|order: &mut Value| order["owner_kind"] = json!("army_group")),
            ),
            (
                "a slot index outside the engine's range",
                Box::new(|order: &mut Value| {
                    order["slot_index"] = json!(D54_SLOT_INDEX_COUNT);
                }),
            ),
            (
                "an owner id outside the engine's range",
                Box::new(|order: &mut Value| {
                    order["army_id"] = json!(u64::from(D54_MAX_ARMY_ID) + 1);
                    order["owner_id"] = json!(u64::from(D54_MAX_ARMY_ID) + 1);
                }),
            ),
            (
                "a zero order instance id",
                Box::new(|order: &mut Value| order["engine_order_instance_id"] = json!(0)),
            ),
            // The order instance bound is NOT the army id bound. Spelling it
            // again here once put it at i32::MAX while the consumer stopped at
            // a hundred million, and nothing in this battery reached past zero.
            (
                "an order instance id outside the engine's range",
                Box::new(|order: &mut Value| {
                    order["engine_order_instance_id"] =
                        json!(u64::from(D54_MAX_ORDER_INSTANCE_ID) + 1);
                }),
            ),
            (
                "an assigned division outside the engine's range",
                Box::new(|order: &mut Value| {
                    order["assigned_division_ids"] = json!([u64::from(D54_MAX_ARMY_ID) + 1]);
                }),
            ),
            (
                "a target state where the contract carries none",
                Box::new(|order: &mut Value| order["target_states"] = json!([7])),
            ),
            (
                "a declared target count where the contract carries null",
                Box::new(|order: &mut Value| order["target_declared_count"] = json!(1)),
            ),
            (
                "a path order with no exact geometry",
                Box::new(|order: &mut Value| order["geometry_status"] = json!("fixture_required")),
            ),
            (
                "a path province outside the map id range",
                Box::new(|order: &mut Value| {
                    order["path_provinces"] = json!([101, u64::from(D54_MAX_MAP_ID) + 1]);
                }),
            ),
            (
                "a target_status a visual order never carries",
                Box::new(|order: &mut Value| order["target_status"] = json!("exact")),
            ),
            (
                "a duplicated assigned division",
                Box::new(|order: &mut Value| order["assigned_division_ids"] = json!([17, 17])),
            ),
            (
                "a zero assigned division",
                Box::new(|order: &mut Value| order["assigned_division_ids"] = json!([0])),
            ),
        ] {
            mutations.push((label.to_owned(), mutate));
        }

        for (label, mutate) in &mutations {
            let (mut order_of_battle, formations) = one_folded_hour();
            mutate(&mut order_of_battle["extra"]["army_battleplans_depth_live"]["orders"][0]);
            let hour = project(&order_of_battle, &formations, &formations, None);
            assert_eq!(
                hour.visual_d54_orders, 0,
                "{label} must leave the census empty"
            );
            assert!(hour.plans.is_empty(), "{label}");
            // And the hour is GATED, because a row parse_order refuses is not
            // a row replay_core drops: it reports the whole view unavailable,
            // so the producer that quietly skipped the row published a plan
            // census for an hour the consumer shows nothing of.
            assert_eq!(
                hour.hour_gates
                    .iter()
                    .map(|gate| gate.recorded_input.as_str())
                    .collect::<Vec<_>>(),
                [D54],
                "{label}"
            );
        }
        assert!(
            mutations.len() >= D54_PINNED_STATUS.len() + D54_BASE_FIELDS.len(),
            "every pin in the contract has to be exercised"
        );
    }

    /// kills: this producer refusing a row the view ADMITS at the boundary.
    ///
    /// The refusal battery walks zero and one past every bound, which catches a
    /// producer that is too loose and cannot catch one that is too tight - and
    /// too tight is the same census mismatch in the other direction: at exactly
    /// the bound replay_core reports the view Present while this producer gates
    /// the hour, so the consumer is shown nothing and the gate names an input
    /// the recording carried. Tightening any `<=` here to `<` fails this.
    ///
    /// The consumer's own edge is pinned separately, by
    /// `army_battleplans.rs::the_admission_boundary_is_exactly_the_shared_producer_constants`.
    #[test]
    fn every_bound_the_consumer_admits_at_is_a_bound_this_producer_mints_at() {
        let (mut order_of_battle, _) = one_folded_hour();
        let order = &mut order_of_battle["extra"]["army_battleplans_depth_live"]["orders"][0];
        order["engine_order_instance_id"] = json!(D54_MAX_ORDER_INSTANCE_ID);
        order["army_id"] = json!(D54_MAX_ARMY_ID);
        order["owner_id"] = json!(D54_MAX_ARMY_ID);
        order["slot_index"] = json!(D54_SLOT_INDEX_COUNT - 1);
        order["path_provinces"] = json!([D54_MAX_MAP_ID, D54_MAX_MAP_ID - 1]);
        order["assigned_division_ids"] = json!([D54_MAX_ARMY_ID]);
        let owners = vec![FormationOwner {
            is_army_group: false,
            owner_id: D54_MAX_ARMY_ID,
            owner_country: "GER".into(),
        }];
        let hour = project(&order_of_battle, &owners, &owners, None);
        assert!(hour.hour_gates.is_empty(), "{:?}", hour.hour_gates);
        assert_eq!(hour.visual_d54_orders, 1);
        assert_eq!(hour.plans.len(), 1, "a row at every bound is still minted");
        let plan = &hour.plans[0];
        assert_eq!(plan.engine_order_instance_id, D54_MAX_ORDER_INSTANCE_ID);
        assert_eq!(plan.owner_id, D54_MAX_ARMY_ID);
        assert_eq!(plan.slot_index, D54_SLOT_INDEX_COUNT - 1);
        assert_eq!(
            plan.route_province_ids,
            [D54_MAX_MAP_ID, D54_MAX_MAP_ID - 1]
        );
        assert_eq!(plan.ordered_assigned_division_ids, [D54_MAX_ARMY_ID]);

        // `target_states` is a SECOND admission site on the map-id bound and
        // the row above cannot reach it: a visual order carries none, so only a
        // type 5 does - a row the view admits and this family does not draw.
        // Refusing it is still refusing the whole view.
        let (mut order_of_battle, formations) = one_folded_hour();
        let order = &mut order_of_battle["extra"]["army_battleplans_depth_live"]["orders"][0];
        order["order_type"] = json!(5);
        order["target_states"] = json!([D54_MAX_MAP_ID, D54_MAX_MAP_ID - 1]);
        order["target_declared_count"] = json!(2);
        order["target_status"] = json!("exact");
        order["path_provinces"] = json!([]);
        let hour = project(&order_of_battle, &formations, &formations, None);
        assert!(
            hour.hour_gates.is_empty(),
            "a type 5 order at the map-id bound is in the view: {:?}",
            hour.hour_gates
        );
        assert_eq!(hour.visual_d54_orders, 0, "and outside this family");
        assert!(hour.plans.is_empty());
    }

    /// kills: a visual order whose geometry d54 could not measure being read as
    /// outside the family.
    ///
    /// `parse_order` ADMITS a type 1-3 order with `geometry_status ==
    /// "fixture_required"` and an empty path, as geometry Unavailable, and the
    /// consumer's visual filter keeps it -- so it is IN the census this
    /// producer's plan count must equal. Treating it as outside made the
    /// producer report a census one short of the one the consumer counts, which
    /// is the same defect as minting an order replay_core refuses, in the other
    /// direction.
    #[test]
    fn a_visual_order_d54_could_not_measure_counts_and_gates_rather_than_vanishing() {
        let (mut order_of_battle, formations) = one_folded_hour();
        let order = &mut order_of_battle["extra"]["army_battleplans_depth_live"]["orders"][0];
        order["geometry_status"] = json!("fixture_required");
        order["path_provinces"] = json!([]);
        let hour = project(&order_of_battle, &formations, &formations, None);
        assert_eq!(
            hour.visual_d54_orders, 1,
            "the order is in the census the consumer counts"
        );
        assert!(hour.plans.is_empty());
        assert_eq!(
            hour.hour_gates
                .iter()
                .map(|gate| gate.recorded_input.as_str())
                .collect::<Vec<_>>(),
            ["orders.army_battleplans_semantic"]
        );
        assert!(hour.hour_gates[0]
            .recorded_state
            .contains("no exact path geometry"));
    }

    /// kills: the visual census reading as a PREFIX, or changing with a row's
    /// position in the hour.
    ///
    /// Preserving the accumulated count across a gate is not enough on its own.
    /// A walk that returns at the first order it cannot handle reports the
    /// orders it had reached, so the same two rows in the other order report a
    /// different whole-hour count - a number that is wrong in the same way a
    /// zero was, and wrong in a way no gate names. Three mixes, each in both
    /// orders: two admitted rows with no formation census, an unmintable
    /// geometry beside a mintable row, and a row the view REFUSES beside a
    /// valid one - the last of which has no census at all rather than a count.
    #[test]
    fn the_visual_census_is_the_whole_hour_whatever_order_its_rows_arrive_in() {
        fn row(mutate: impl Fn(&mut Value)) -> Value {
            let (order_of_battle, _) = one_folded_hour();
            let mut row = order_of_battle["extra"]["army_battleplans_depth_live"]["orders"][0]
                .clone();
            mutate(&mut row);
            row
        }
        let first = || row(|_| {});
        let second = || {
            row(|order| {
                order["engine_order_instance_id"] = json!(2026);
                order["army_id"] = json!(9);
                order["owner_id"] = json!(9);
            })
        };
        let unmeasured = || {
            row(|order| {
                order["engine_order_instance_id"] = json!(2027);
                order["geometry_status"] = json!("fixture_required");
                order["path_provinces"] = json!([]);
            })
        };
        // Both owners are in the manager's identity census on every case
        // below: what varies is whether their COUNTRIES resolved, which is the
        // failure this test is about.
        let owners = vec![
            FormationOwner {
                is_army_group: false,
                owner_id: 5,
                owner_country: "GER".into(),
            },
            FormationOwner {
                is_army_group: false,
                owner_id: 9,
                owner_country: "GER".into(),
            },
        ];
        let hour_of = |rows: Vec<Value>,
                       formations: &[FormationOwner],
                       gate: Option<&'static str>| {
            let (mut order_of_battle, _) = one_folded_hour();
            order_of_battle["extra"]["army_battleplans_depth_live"]["orders"] = Value::Array(rows);
            project(&order_of_battle, &owners, formations, gate)
        };

        // Two rows the view admits, and no formation census to mint them
        // against. Both are in the hour's visual census whichever is walked
        // first, and the gate is the one XMAP13 named.
        for rows in [vec![first(), second()], vec![second(), first()]] {
            let hour = hour_of(rows, &[], Some(DIVISIONS));
            assert_eq!(hour.visual_d54_orders, 2, "both orders are in the census");
            assert!(hour.plans.is_empty());
            assert_eq!(
                hour.hour_gates
                    .iter()
                    .map(|gate| gate.recorded_input.as_str())
                    .collect::<Vec<_>>(),
                [DIVISIONS]
            );
            let reason = hour.gate().unavailable_reason();
            assert!(reason.contains("0 plan rows against 2 visual"), "{reason}");
        }

        // A visual row d54 could not measure beside one it could. The VIEW
        // stands, so both rows are counted, and the gate is the geometry the
        // consumer refuses the frame at - in either order.
        for rows in [
            vec![unmeasured(), first()],
            vec![first(), unmeasured()],
        ] {
            let hour = hour_of(rows, &owners, None);
            assert_eq!(hour.visual_d54_orders, 2, "the view stands and holds both");
            assert!(hour.plans.is_empty());
            assert_eq!(
                hour.hour_gates
                    .iter()
                    .map(|gate| gate.recorded_input.as_str())
                    .collect::<Vec<_>>(),
                [D54]
            );
            assert!(hour.hour_gates[0]
                .recorded_state
                .contains("no exact path geometry"));
        }

        // One row the view REFUSES beside one it admits. The view is gone
        // either way, so the hour has no visual census to count rather than a
        // count that depends on which row came first.
        for rows in [
            vec![Value::Null, first()],
            vec![first(), Value::Null],
        ] {
            let hour = hour_of(rows, &owners, None);
            assert_eq!(
                hour.visual_d54_orders, 0,
                "an hour with no view has no visual census"
            );
            assert!(hour.plans.is_empty());
            assert_eq!(
                hour.hour_gates
                    .iter()
                    .map(|gate| gate.recorded_input.as_str())
                    .collect::<Vec<_>>(),
                [D54]
            );
        }
    }

    /// kills: two rows carrying one stable identity being minted as two plans.
    ///
    /// replay_core keys orders on owner, instance id and generation, and
    /// reports the whole view unavailable for a repeat. Slot index is NOT in
    /// that key, which is what makes this a different check from the field
    /// pins: the two rows here differ, and still collide.
    #[test]
    fn two_orders_with_one_stable_identity_cost_the_whole_view() {
        let (mut order_of_battle, formations) = one_folded_hour();
        let orders = order_of_battle["extra"]["army_battleplans_depth_live"]["orders"]
            .as_array_mut()
            .expect("orders");
        let mut twin = orders[0].clone();
        twin["slot_index"] = json!(2);
        twin["path_provinces"] = json!([103, 104]);
        orders.push(twin);
        let hour = project(&order_of_battle, &formations, &formations, None);
        assert!(hour.plans.is_empty());
        assert_eq!(
            hour.hour_gates
                .iter()
                .map(|gate| gate.recorded_input.as_str())
                .collect::<Vec<_>>(),
            [D54]
        );
        assert!(hour.hour_gates[0].recorded_state.contains("one stable identity"));
    }

    /// kills: an order type outside the visual set being counted as missing
    /// rather than as outside the family, and: "outside the family" being read
    /// off the order type alone.
    ///
    /// A non-visual row is only outside this family while the VIEW still admits
    /// it. Both rows here are admitted by `parse_order` - a type 5 target-state
    /// order and a type 7 unsupported one, each carrying the geometry its own
    /// type pins - so the hour stands with an empty visual census. Change
    /// either row's pins and it stops being outside the family and starts
    /// costing the whole view, which the refusal battery covers.
    #[test]
    fn a_non_visual_order_the_view_admits_is_outside_this_family_and_not_a_gap_in_it() {
        for (label, mutate) in [
            (
                "a type 5 target-state order",
                Box::new(|order: &mut Value| {
                    order["order_type"] = json!(5);
                    order["target_states"] = json!([7, 9]);
                    order["target_declared_count"] = json!(2);
                    order["target_status"] = json!("exact");
                    order["path_provinces"] = json!([]);
                }) as Box<dyn Fn(&mut Value)>,
            ),
            (
                "a type 7 order this replay has no geometry for",
                Box::new(|order: &mut Value| {
                    order["order_type"] = json!(7);
                    order["path_provinces"] = json!([]);
                    order["geometry_status"] = json!("fixture_required");
                }),
            ),
        ] {
            let (mut order_of_battle, formations) = one_folded_hour();
            mutate(&mut order_of_battle["extra"]["army_battleplans_depth_live"]["orders"][0]);
            let hour = project(&order_of_battle, &formations, &formations, None);
            assert!(hour.hour_gates.is_empty(), "{label}: {:?}", hour.hour_gates);
            assert_eq!(hour.visual_d54_orders, 0, "{label}");
            assert!(hour.plans.is_empty(), "{label}");
        }
    }

    /// kills: a plan owner with no formation minting a country of the
    /// producer's choosing, and: the gate naming the wrong input for it.
    ///
    /// There is no census here to be outside of - XMAP13 gated it - and the
    /// input that names is XMAP13's own: an ARMY's country comes from its
    /// assigned divisions and an ARMY GROUP's from the theatre selector, so a
    /// gate this producer picked for itself would send a reader at the wrong
    /// one half the time.
    #[test]
    fn a_plan_owner_with_no_formation_census_gates_the_input_xmap13_named() {
        for (is_army_group, expected_input) in [(false, DIVISIONS), (true, HIERARCHY)] {
            let (mut order_of_battle, _) = one_folded_hour();
            if is_army_group {
                let order =
                    &mut order_of_battle["extra"]["army_battleplans_depth_live"]["orders"][0];
                order["owner_kind"] = json!("army_group");
                order["army_id"] = Value::Null;
            }
            // The owner IS in the manager's identity census - the view stands -
            // and it is only the country that has no source.
            let owners = vec![FormationOwner {
                is_army_group,
                owner_id: 5,
                owner_country: String::new(),
            }];
            let hour = project(&order_of_battle, &owners, &[], Some(expected_input));
            assert_eq!(
                hour.hour_gates
                    .iter()
                    .map(|gate| gate.recorded_input.as_str())
                    .collect::<Vec<_>>(),
                [expected_input]
            );
            assert!(hour.plans.is_empty(), "a gated hour mints no census at all");
            // The VISUAL count survives the gate. It is a measurement of what
            // d54 carried, not a claim about what was minted, and zeroing it
            // made the gate say the hour had no visual orders when it had one
            // it could not mint - the two facts the gate exists to separate.
            assert_eq!(hour.visual_d54_orders, 1);
            let gate = hour.gate();
            gate.validate().expect("a well-formed gate");
            let reason = gate.unavailable_reason();
            assert!(reason.contains("0 plan rows against 1 visual"), "{reason}");
        }
    }

    /// kills: an owner the MANAGER never named being reported as a missing
    /// country.
    ///
    /// When XMAP13's formation census stands, it IS the manager's complete
    /// identity census - so an owner outside it is an owner replay_core reports
    /// the whole view unavailable for, and naming `units.divisions` there would
    /// send a reader at a section that is not missing anything.
    #[test]
    fn an_owner_outside_a_complete_formation_census_is_a_view_the_replay_refuses() {
        let (order_of_battle, formations) = one_folded_hour();
        let other = vec![FormationOwner {
            is_army_group: false,
            owner_id: 6,
            owner_country: "GER".into(),
        }];
        assert_eq!(formations[0].owner_id, 5, "the order's owner");
        let hour = project(&order_of_battle, &other, &other, None);
        assert_eq!(
            hour.visual_d54_orders, 0,
            "an hour whose view replay_core refuses has no census to have counted"
        );
        assert!(hour.plans.is_empty());
        assert_eq!(
            hour.hour_gates
                .iter()
                .map(|gate| gate.recorded_input.as_str())
                .collect::<Vec<_>>(),
            [D54]
        );
        assert!(hour.hour_gates[0]
            .recorded_state
            .contains("absent from the orders-group manager identity census"));
    }

    /// kills: an unreadable MANAGER being reported as a missing country, and:
    /// the visual census being counted for an hour with no view.
    ///
    /// replay_core admits every d54 order row against the manager's identity
    /// census, so a manager it refuses is a view it refuses - whatever the
    /// orders section holds. That is not the same failure as a manager it read
    /// whose owners have no country, where the view stands, and one
    /// `Option<&str>` gate could not tell the two apart: both gate, and both
    /// name d54.
    #[test]
    fn a_manager_the_replay_refuses_is_a_view_it_refuses_and_not_a_missing_country() {
        let (order_of_battle, formations) = one_folded_hour();
        let hour = project_fold_sections(
            17,
            &order_of_battle,
            &Xmap13Formations {
                owner_census: None,
                with_countries: &formations,
                gate: Some(D54),
            },
        );
        assert_eq!(
            hour.visual_d54_orders, 0,
            "an hour whose manager the replay refuses has no view to count"
        );
        assert!(hour.plans.is_empty());
        assert_eq!(
            hour.hour_gates
                .iter()
                .map(|gate| gate.recorded_input.as_str())
                .collect::<Vec<_>>(),
            [D54]
        );
        assert!(hour.hour_gates[0]
            .recorded_state
            .contains("refuses the orders-group manager"));
    }

    /// kills: owner admission being asked of the COUNTRIES rather than of the
    /// identity census.
    ///
    /// The two fail independently. An hour whose countries are gated still has
    /// a manager census, and replay_core still refuses the whole view for an
    /// order whose owner is not in it - so a producer that skipped admission
    /// whenever the countries were missing reported a visual census, and a
    /// country gate, for an hour the consumer shows nothing of.
    #[test]
    fn an_orphan_owner_costs_the_view_even_where_the_countries_are_gated() {
        let (order_of_battle, _) = one_folded_hour();
        let owners = vec![FormationOwner {
            is_army_group: false,
            owner_id: 6,
            owner_country: String::new(),
        }];
        let hour = project(&order_of_battle, &owners, &[], Some(DIVISIONS));
        assert_eq!(hour.visual_d54_orders, 0, "the order's owner is 5, not 6");
        assert!(hour.plans.is_empty());
        assert_eq!(
            hour.hour_gates
                .iter()
                .map(|gate| gate.recorded_input.as_str())
                .collect::<Vec<_>>(),
            [D54],
            "the cause is the view, not the country"
        );
    }

    /// kills: an hour with no formation census publishing an empty plan census
    /// as if it were a measurement.
    ///
    /// A gate that only fires when a visual order happens to need a country
    /// leaves the quiet hours - a standing d54 view with nothing visual in it -
    /// claiming a complete zero-plan census while XMAP13 is unavailable, which
    /// is a claim no reader can use and none of the hour's gates contradict.
    #[test]
    fn a_gated_formation_census_gates_this_hour_even_with_nothing_visual_in_it() {
        let (mut order_of_battle, _) = one_folded_hour();
        let order = &mut order_of_battle["extra"]["army_battleplans_depth_live"]["orders"][0];
        order["order_type"] = json!(7);
        order["path_provinces"] = json!([]);
        order["geometry_status"] = json!("fixture_required");
        let owners = vec![FormationOwner {
            is_army_group: false,
            owner_id: 5,
            owner_country: String::new(),
        }];
        let hour = project(&order_of_battle, &owners, &[], Some(HIERARCHY));
        assert_eq!(hour.visual_d54_orders, 0, "the view stands and holds nothing");
        assert!(hour.plans.is_empty());
        assert_eq!(
            hour.hour_gates
                .iter()
                .map(|gate| gate.recorded_input.as_str())
                .collect::<Vec<_>>(),
            [HIERARCHY]
        );
    }

    /// kills: an incomplete d54 view being minted from.
    #[test]
    fn a_d54_view_that_is_not_zero_omission_gates_rather_than_minting_part_of_it() {
        for key in [
            "baseline_seen",
            "omitted_non_exact_rows_this_hour",
            "path_geometry_omitted_rows_this_hour",
            "identity_or_active_omitted_rows_this_hour",
        ] {
            let (mut order_of_battle, formations) = one_folded_hour();
            order_of_battle["extra"]["army_battleplans_depth_live"][key] = if key == "baseline_seen"
            {
                json!(false)
            } else {
                json!(1)
            };
            let hour = project(&order_of_battle, &formations, &formations, None);
            assert_eq!(
                hour.hour_gates
                    .iter()
                    .map(|gate| gate.recorded_input.as_str())
                    .collect::<Vec<_>>(),
                ["orders.army_battleplans_semantic"],
                "{key}"
            );
        }
    }
}
