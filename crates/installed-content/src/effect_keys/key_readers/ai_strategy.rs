//! CAddAIStrategyEffect (1403AE990), with CAIStrategyReader (1406543E0).
//! Native enum and helper-key data comes from
//! captures/astra/effect-key-readers-2/readers_high/ai.py, using hoi4re.

use crate::{
    combat_tactics, crt, fixed5,
    key_names::KeyNames,
    parser::native::{Field, Shape, Token},
};

#[derive(Clone, Copy)]
pub(crate) enum Node {
    Strategy,
    Trigger(combat_tactics::Node),
}

#[derive(Clone, Copy)]
enum Helper {
    Market,
    Scorch,
    Buffer,
    Targeted,
    Front,
    Mission,
    Operation,
}

#[derive(Clone, Copy)]
enum Context {
    Allocated(Helper),
    /// 140650810 returns null for a negative global index.
    Null,
    Unknown(&'static str),
}

#[derive(Clone, Copy)]
enum ValueReader {
    One,
    Values,
    Tag,
    State,
    Region,
    Area,
    Trigger,
    OrderType,
    ExecutionType,
}

// BEGIN EXTRACTED ENUMS
// Strategy types: 141497260.
const TYPES: &[(&str, i32)] = &[
    ("activate_crypto", 0x43),
    ("added_military_to_civilian_factory_ratio", 0x32),
    ("agency_ai_base_num_factories_factor", 0x4d),
    ("agency_ai_per_upgrade_factories_factor", 0x4e),
    ("ai_wanted_divisions_factor", 0x60),
    ("air_factory_balance", 0x17),
    ("air_xp_spend_priority", 0x13),
    ("alliance", 0x6),
    ("antagonize", 0x1),
    ("area_priority", 0x26),
    ("asking_foreign_garrison", 0x4c),
    ("avoid_starting_wars", 0x5f),
    ("become_head_of_counter_intel", 0x6a),
    ("become_head_of_crypto", 0x68),
    ("become_head_of_operations", 0x69),
    ("become_spymaster", 0x67),
    ("befriend", 0x2),
    ("build_airplane", 0x8),
    ("build_army", 0x9),
    ("build_building", 0xa),
    ("build_ship", 0x7),
    ("building_target", 0x3c),
    ("coast_defense", 0x6f),
    ("conquer", 0x0),
    ("consider_weak", 0x4),
    ("contain", 0xd),
    ("convoy_efficiency_to_cancel_trades", 0x3a),
    ("convoy_raiding_target", 0x6c),
    ("declare_war", 0x2e),
    ("decrypt_target", 0x40),
    ("diplo_action_acceptance", 0x4a),
    ("diplo_action_desire", 0x49),
    ("dockyard_to_military_factory_ratio", 0x35),
    ("dont_defend_ally_borders", 0x36),
    ("dont_join_wars_with", 0x31),
    ("equipment_market_buy", 0x23),
    ("equipment_market_buying_threshold", 0x22),
    ("equipment_market_for_sale_factor", 0x1f),
    ("equipment_market_for_sale_threshold", 0x1e),
    ("equipment_market_max_for_sale", 0x20),
    ("equipment_market_min_for_sale", 0x21),
    ("equipment_market_spend_factories", 0x1d),
    ("equipment_market_trade_desire", 0x24),
    ("equipment_production_factor", 0x18),
    ("equipment_production_min_factories", 0x1b),
    ("equipment_production_min_factories_archetype", 0x1c),
    ("equipment_production_surplus_management", 0x1a),
    ("equipment_stockpile_surplus_ratio", 0x16),
    ("equipment_variant_production_factor", 0x19),
    ("factory_build_score_factor", 0x33),
    ("force_build_armies", 0x3f),
    ("force_concentration_factor", 0x63),
    ("force_concentration_front_factor", 0x62),
    ("force_concentration_target_weight", 0x64),
    ("force_defend_ally_borders", 0x37),
    ("front_armor_score", 0x53),
    ("front_control", 0x4f),
    ("front_unit_request", 0x48),
    ("fuel_buffer", 0x39),
    ("garrison", 0xc),
    ("garrison_reinforcement_priority", 0x50),
    ("ignore", 0x25),
    ("ignore_army_incompetence", 0x2f),
    ("ignore_claim", 0x34),
    ("influence", 0x5),
    ("intelligence_agency_branch_desire_factor", 0x45),
    ("intelligence_agency_usable_factories", 0x44),
    ("invade", 0xe),
    ("invasion_unit_request", 0x47),
    ("land_xp_spend_priority", 0x11),
    ("min_convoy_efficiency_factor_for_war_support_hit", 0x3b),
    ("min_wanted_supply_trains", 0x57),
    ("min_wanted_supply_trucks", 0x55),
    ("naval_avoid_region", 0x2a),
    ("naval_blockade", 0x6e),
    ("naval_convoy_raid_region", 0x59),
    ("naval_dominance", 0x6b),
    ("naval_invasion_dominance_weight", 0x5c),
    ("naval_invasion_focus", 0x5b),
    ("naval_invasion_support_priority", 0x5d),
    ("naval_mission_threshold", 0x38),
    ("navy_xp_spend_priority", 0x12),
    ("occupation_policy", 0x28),
    ("operation_equipment_priority", 0x52),
    ("operative_mission", 0x4b),
    ("operative_operation", 0x51),
    ("pp_spend_amount", 0x2d),
    ("pp_spend_priority", 0x14),
    ("prepare_for_war", 0x2b),
    ("production_upgrade_desire_offset", 0x3e),
    ("protect", 0x3),
    ("put_unit_buffers", 0x46),
    ("raid_target_country", 0x65),
    ("railway_gun_divisions_ratio", 0x5a),
    ("recruit_scientist", 0x66),
    ("research_tech", 0xb),
    ("research_weight_factor", 0x61),
    ("role_ratio", 0x10),
    ("save_equipment", 0x2c),
    ("scorched_earth_prio", 0x42),
    ("send_lend_lease_desire", 0x41),
    ("send_volunteers_desire", 0x27),
    ("spare_unit_factor", 0x30),
    ("spent_faction_initiative_priority", 0x6d),
    ("strategic_air_importance", 0x5e),
    ("strike_force_home_base", 0x3d),
    ("support", 0x29),
    ("template_prio", 0x15),
    ("theatre_distribution_demand_increase", 0x54),
    ("unit_ratio", 0xf),
    ("wanted_supply_trains", 0x58),
    ("wanted_supply_trucks", 0x56),
];
// 140FAEFE0.
const MISSIONS: &[&str] = &[
    "boost_ideology",
    "build_intel_network",
    "control_trade",
    "counter_intelligence",
    "diplomatic_pressure",
    "no_mission",
    "propaganda",
    "quiet_network",
    "root_out_resistance",
];
// 141498240.
const EXECUTION_TYPES: &[&str] = &["balanced", "careful", "rush", "rush_weak"];
// 1406559B0.
const ORDER_TYPES: &[&str] = &["front", "invasion"];
// 140530BB0.
const SELECTORS: &[&str] = &[
    "capital",
    "controller",
    "from",
    "occupied",
    "owner",
    "prev",
    "random",
    "root",
    "this",
];
// END EXTRACTED ENUMS

// CAIEquipmentMarketBuyStrategyData::vf1 (1406538D0).
const MARKET: &[(&str, ValueReader)] = &[
    ("equipment_type", ValueReader::One),
    ("seller", ValueReader::One),
];
// CScorchEarthPrioData::vf1 (140655AC0).
const SCORCH: &[(&str, ValueReader)] = &[("states", ValueReader::Values)];
// CBufferUnitsAtStateData::vf1 (140655760).
const BUFFER: &[(&str, ValueReader)] = &[
    ("states", ValueReader::Values),
    ("ratio", ValueReader::One),
    ("area", ValueReader::Area),
    ("order_id", ValueReader::One),
    ("subtract_invasions_from_need", ValueReader::One),
    ("subtract_fronts_from_need", ValueReader::One),
];
// CTargetedStrategyData::vf1 (1406565D0), also Front's fallback.
const TARGETED: &[(&str, ValueReader)] = &[
    ("country", ValueReader::Tag),
    ("tag", ValueReader::Tag),
    ("state", ValueReader::State),
    ("ratio", ValueReader::One),
    ("area", ValueReader::Area),
    ("strategic_region", ValueReader::Region),
    ("state_trigger", ValueReader::Trigger),
    ("country_trigger", ValueReader::Trigger),
];
// CFrontControlStrategyData::vf1 (1406559B0).
const FRONT: &[(&str, ValueReader)] = &[
    ("priority", ValueReader::One),
    ("ordertype", ValueReader::OrderType),
    ("execute_order", ValueReader::One),
    ("execution_type", ValueReader::ExecutionType),
    ("manual_attack", ValueReader::One),
];
// CAIOperativeMissionStrategy::vf1 (1406539E0).
const MISSION: &[(&str, ValueReader)] = &[
    ("priority", ValueReader::One),
    ("state", ValueReader::One),
    ("mission", ValueReader::One),
    ("mission_target", ValueReader::One),
    ("num_operatives", ValueReader::One),
];
// CAIOperativeOperationStrategy::vf1 (140653B70).
const OPERATION: &[(&str, ValueReader)] = &[
    ("priority", ValueReader::One),
    ("state", ValueReader::One),
    ("region", ValueReader::One),
    ("operation", ValueReader::One),
    ("operation_target", ValueReader::One),
];

fn helper(kind: i32) -> Option<Helper> {
    // Allocation switch 14063AC10 and fallback switch 1406543E0.
    match kind {
        0x23 => Some(Helper::Market),
        0x42 => Some(Helper::Scorch),
        0x46 => Some(Helper::Buffer),
        0x47 | 0x48 | 0x62 | 0x64 => Some(Helper::Targeted),
        0x4B => Some(Helper::Mission),
        0x4F => Some(Helper::Front),
        0x51 => Some(Helper::Operation),
        _ => None,
    }
}

fn token_is_one_of(token: &Token, words: &[&str]) -> bool {
    words.iter().any(|word| token.is_keyword(word))
}

fn accepted(yes: bool) -> Shape<Node> {
    if yes { Shape::One } else { Shape::Skip }
}

pub(super) struct Shaper<'n> {
    names: Option<&'n KeyNames>,
    triggers: combat_tactics::Shaper<'n>,
    kind: i32,
    context: Context,
}

impl<'n> Shaper<'n> {
    pub(super) fn new(names: Option<&'n KeyNames>) -> Self {
        // Factory 14033AE60 initializes type and global helper index to 0.
        // Global slot 0 may hold a helper allocated by an earlier reader.
        Self {
            names,
            triggers: combat_tactics::Shaper::new(names),
            kind: 0,
            context: Context::Unknown("initial global strategy-specific index 0"),
        }
    }

    pub(super) fn shape(&mut self, node: Node, field: &Field<'_>) -> Shape<Node> {
        if let Node::Trigger(node) = node {
            return self.triggers.shape(node, field).map(Node::Trigger);
        }
        match field.keyword().as_deref() {
            Some(b"type") => {
                // 141497260 matches token IDs, including no quoted names.
                if let Some((_, kind)) = TYPES.iter().find(|(word, _)| field.value.is_keyword(word))
                {
                    self.kind = *kind;
                    if let Some(helper) = helper(*kind) {
                        self.context = Context::Allocated(helper);
                    }
                } else if let Some(kind) = crt::scan_i32(field.value.c_str()).stored() {
                    // ReadScriptSignedInt32 (1424AB970 -> 1424B02C0)
                    // preserves type on failed conversion and never
                    // allocates or resets the existing helper index.
                    self.kind = kind;
                }
                Shape::One
            }
            Some(b"value") => Shape::One,
            Some(b"id") => {
                // The effect tries ParseScopeSelector (140530BB0) first:
                // nine native tokens, or a case-sensitive text prefix.
                if token_is_one_of(field.value, SELECTORS)
                    || field.value.c_str().starts_with(b"event_target:")
                {
                    return Shape::One;
                }
                if self.kind == 0x4B {
                    // 140FAEFE0 failure calls RecordError (1424ACF90).
                    return accepted(token_is_one_of(field.value, MISSIONS));
                }
                if self.kind == 0x6B && field.value.is_number() {
                    self.set_index(fixed5::decimal_i32(field.value.c_str()));
                }
                // Other id validations only log/report without a skip
                // (1424ACD40 / 1424ACEE0); scoped readers are one value.
                Shape::One
            }
            Some(b"target") => {
                // 1424B0160 tests digits in the text, even if quoted;
                // otherwise area/global lexer token lookups can change
                // the helper index. Their global numeric index is not
                // determined by this effect's local reader state.
                let text = field.value.c_str();
                if !text.is_empty() && text.iter().all(u8::is_ascii_digit) {
                    if let Some(index) = crt::scan_u32(text).stored() {
                        self.set_index(index as i32);
                    }
                } else {
                    self.context = Context::Unknown("target area or global lexer token lookup");
                }
                Shape::One
            }
            Some(b"target_country") => {
                // 14070F050 exact tag lookup, then ParseTokenInt32
                // compared with the live country database size. A
                // negative result is always below that size and gives
                // a proven null helper (140650810).
                let value = fixed5::decimal_i32(field.value.c_str());
                if self
                    .names
                    .is_some_and(|names| names.countries.contains(field.value.c_str()))
                {
                    self.context = Context::Unknown("target_country global country index");
                } else if value < 0 && self.names.is_some() {
                    self.context = Context::Null;
                } else {
                    self.context = Context::Unknown("target_country global index lookup");
                }
                Shape::One
            }
            _ => self.specific(field),
        }
    }

    fn set_index(&mut self, index: i32) {
        self.context = if index < 0 {
            Context::Null
        } else {
            Context::Unknown("explicit global strategy-specific index")
        };
    }

    fn specific(&self, field: &Field<'_>) -> Shape<Node> {
        if helper(self.kind).is_none() {
            return Shape::Skip;
        }
        let context = match self.context {
            Context::Allocated(helper) => helper,
            Context::Null => return Shape::Skip,
            Context::Unknown(cause) => {
                return Shape::Unsettled(format!(
                    "CAddAIStrategyEffect 1406543E0: {cause} needs the global strategy-specific data table (140650810)"
                ));
            }
        };
        let keys = match context {
            Helper::Market => MARKET,
            Helper::Scorch => SCORCH,
            Helper::Buffer => BUFFER,
            Helper::Targeted => TARGETED,
            Helper::Front => FRONT,
            Helper::Mission => MISSION,
            Helper::Operation => OPERATION,
        };
        let keyword = field.keyword();
        let key = keyword.as_deref();
        let own = keys.iter().find(|(word, _)| key == Some(word.as_bytes()));
        let value_reader = own.or_else(|| {
            if matches!(context, Helper::Front) {
                TARGETED
                    .iter()
                    .find(|(word, _)| key == Some(word.as_bytes()))
            } else {
                None
            }
        });
        let Some((_, reader)) = value_reader else {
            // False from the helper reaches outer ReportUnexpectedToken.
            // Market (1406538D0) reports too. The first skip leaves
            // CURRENT on Close/End, so the second error skip consumes
            // nothing (1424AD270); both have this one skip's consumption.
            return Shape::Skip;
        };
        match reader {
            ValueReader::One => Shape::One,
            ValueReader::Values => Shape::ValueList,
            // CAndTrigger::LoadBody is unconditional and has no repeated
            // child gate here (1406565D0), unlike effect limit fields.
            ValueReader::Trigger => Shape::Body(Node::Trigger(combat_tactics::Node::Trigger)),
            ValueReader::OrderType => accepted(token_is_one_of(field.value, ORDER_TYPES)),
            ValueReader::ExecutionType => accepted(token_is_one_of(field.value, EXECUTION_TYPES)),
            ValueReader::Tag => self.tag(field),
            ValueReader::State => self.index(field, false),
            ValueReader::Region => self.index(field, true),
            ValueReader::Area => self.area(field),
        }
    }

    fn tag(&self, field: &Field<'_>) -> Shape<Node> {
        // CCountryTag 140BA4CA0 -> 14070F050, accepted only above index 0.
        if !field.skips_block() {
            return Shape::Skip;
        }
        let Some(names) = self.names else {
            return Shape::Unsettled(
                "CAddAIStrategyEffect 1406565D0: country tags were not supplied".into(),
            );
        };
        accepted(names.countries.contains(field.value.c_str()))
    }

    fn area(&self, field: &Field<'_>) -> Shape<Node> {
        // 14061A540: accepted One and error Skip consume a scalar alike.
        if !field.skips_block() {
            return Shape::Skip;
        }
        Shape::Reported {
            report: "Deferred effect CAddAIStrategyEffect AI area (14061A540): block consumption needs the AI-area registry".into(),
            read: Box::new(Shape::Unsettled("AI area block needs the unavailable registry".into())),
        }
    }

    fn index(&self, field: &Field<'_>, region: bool) -> Shape<Node> {
        // 1406565D0: indexed hit reads one, miss skips CURRENT. Scalars
        // consume identically; only an opening CURRENT needs the roster.
        if !field.skips_block() {
            return Shape::Skip;
        }
        let what = if region { "strategic region" } else { "state" };
        Shape::Reported {
            report: format!(
                "Deferred effect CAddAIStrategyEffect {what} (1406565D0): block consumption needs the indexed registry"
            ),
            read: Box::new(Shape::Unsettled(format!(
                "AI strategy {what} block needs the unavailable registry"
            ))),
        }
    }
}
