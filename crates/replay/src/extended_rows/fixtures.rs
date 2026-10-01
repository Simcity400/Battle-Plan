use super::*;
use std::fmt::Debug;
trait Sample {
    fn sample(tick: usize) -> Self;
}
macro_rules! signed_samples {
    ($($ty:ty),+) => { $(impl Sample for $ty {
        fn sample(tick: usize) -> Self { match tick % 3 { 0 => <$ty>::MIN, 1 => -1, _ => <$ty>::MAX } }
    })+ };
}
macro_rules! unsigned_samples {
    ($($ty:ty),+) => { $(impl Sample for $ty {
        fn sample(tick: usize) -> Self { match tick % 3 { 0 => 0, 1 => 1, _ => <$ty>::MAX } }
    })+ };
}
signed_samples!(i8, i16, i32, i64);
unsigned_samples!(u8, u16, u32, u64);
impl Sample for bool {
    fn sample(tick: usize) -> Self {
        tick.is_multiple_of(2)
    }
}
impl<T: Sample> Sample for Option<T> {
    fn sample(tick: usize) -> Self {
        if tick.is_multiple_of(4) {
            None
        } else {
            Some(T::sample(tick))
        }
    }
}
impl<T: Sample> Sample for Vec<T> {
    fn sample(tick: usize) -> Self {
        (0..tick % 3).map(|index| T::sample(tick + index)).collect()
    }
}
impl<T: Sample, const N: usize> Sample for [T; N] {
    fn sample(tick: usize) -> Self {
        std::array::from_fn(|index| T::sample(tick + index))
    }
}
macro_rules! sample_struct {
    ($ty:ty { $($field:ident : $field_ty:ty),* $(,)? }) => {
        impl Sample for $ty { fn sample(tick: usize) -> Self { Self { $($field: <$field_ty as Sample>::sample(tick),)* } } }
    };
}
trait Fixture: RowCodec + Debug {
    fn fixture(key: u128, tick: usize) -> Self;
    /// The largest valid key; below the key width when an enumerated key
    /// field leaves bit patterns unused.
    fn largest_key() -> u128 {
        maximum_key::<Self>()
    }
}
macro_rules! fixture {
    ($ty:ident, |$key:ident| { $($key_field:ident: $key_value:expr),* $(,)? },
     { $($field:ident: $field_ty:ty),+ $(,)? }) => {
        impl Fixture for $ty {
            fn fixture($key: u128, tick: usize) -> Self {
                Self { $($key_field: $key_value,)* $($field: <$field_ty as Sample>::sample(tick),)+ }
            }
        }
    };
}
sample_struct!(ProjectState {
    current_offset: u32,
    state_enums: [u32; 4],
    not_completed: u8,
    phase_progress: i64,
    phase_target: i32,
    prototype_progress: i32,
    iterations: i32
});
sample_struct!(ProjectContext { province: i32, progress: i32, character_reference: u64, country_present: u8, country_reference: Option<u32>, scientist_present: u8, scientist_reference: Option<u32>, state_present: u8, state_reference: Option<u32> });
sample_struct!(RaidSource { kind: Option<u32>, kind_present: u8, province: u32, ship_reference_type: u32, ship_reference_id: u32, country: i32, building_type: u32, building_location: i32, building_country: i32 });
sample_struct!(RaidScope { province: u32, state: u32, leader_reference: u64, leader_province: u32, building_type: u32, building_location: u32, building_flags: u32, building_country: i32, leader_country: Option<i32> });
sample_struct!(ScriptVariableEntry { name: Vec<u8>, value_raw: i64 });
sample_struct!(AceDeath {
    killer_ace: u64,
    killer_country: u32
});
sample_struct!(AirHistoryEntry {
    side: u8,
    date: i32,
    wing_id: u32,
    classification: u8,
    region_owner: u32,
    planes: i32,
    mission_mask: u32,
    selection: u8,
    copied: u8,
    attributed_country: u32
});
sample_struct!(AirDamageEntry {
    side: u8,
    date: i32,
    wing_id: u32,
    classification: u8,
    channel: u8,
    cause: u32,
    amount: i64
});
sample_struct!(SunkShipEntry { ship_name: Vec<u8>, attribution_name: Vec<u8>, class_name: Vec<u8>, attribution_class_name: Vec<u8>, victim_country: u32, attacker_country: u32, sink_hour: i32, class_token: u32, attribution_class_token: u32, attribution_class_present: u8, model_level: i32, victim_equipment_variant: u64, damage_source_reference: u64, province: u32, province_present: u8, battle: u64, cause_record: u8, assisted_kill: u8 });
fixture!(FocusRow, |key| {country: key as u32}, {tree: Option<Vec<u8>>, current: Option<Vec<u8>>, continuous: Option<Vec<u8>>, progress: i64, paused: u8, completed_count: i32});
fixture!(CompletedFocusRow, |key| {country: (key >> 32) as u32, ordinal: key as u32}, {token: Vec<u8>});
fixture!(ResearchTechnologyRow, |key| {country: (key >> 32) as u32, technology_handle: key as u32}, {classifier: u32, level: i32, researched: u8, progress_raw: i64, ahead_reduction_years_raw: i64});
impl Fixture for ResearchModifierRow {
    fn fixture(key: u128, tick: usize) -> Self {
        Self {
            country: (key >> 64) as u32,
            index: (key >> 32) as u32,
            ordinal: key as u32,
            technology_handle: u32::sample(tick),
            kind: (tick % (Self::SHARING_MISSING_MEMBER as usize + 1)) as u8,
            raw: i64::sample(tick),
            auxiliary: i64::sample(tick + 1),
            count: u32::sample(tick),
            source_id: u32::sample(tick),
            name: Vec::<u8>::sample(tick),
        }
    }
}
impl Fixture for CountryStabilityInputRow {
    fn fixture(key: u128, tick: usize) -> Self {
        let totals = |offset: usize| {
            let tracked = !(tick + offset).is_multiple_of(3);
            (
                tracked.then(|| i64::sample(tick + offset)),
                if tracked { offset as u32 * 7 } else { u32::MAX },
            )
        };
        let parts: [(Option<i64>, u32); 10] = std::array::from_fn(totals);
        // Only the first eight keys have totals here.
        let mut row = Self {
            country: key as u32,
            stability_raw: i64::sample(tick),
            war_support_raw: i64::sample(tick + 1),
            at_war: (tick % 2) as u8,
            offensive_war: (tick / 2 % 2) as u8,
            defensive_war: (tick / 4 % 2) as u8,
            ruling_party_popularity_raw: i64::sample(tick + 2),
            propaganda_stability_q15: i64::sample(tick + 3),
            coastal_protection_stability_raw: i64::sample(tick + 4),
            bombing_war_support_q15: i64::sample(tick + 5),
            heroes_war_support_q15: i64::sample(tick + 6),
            convoys_war_support_q15: i64::sample(tick + 7),
            propaganda_war_support_q15: i64::sample(tick + 8),
            interception_sum_raw: i64::sample(tick + 9),
            world_tension_raw: i64::sample(tick + 10),
            base_stability_war_factor: i64::sample(tick + 11),
            base_stability_party_popularity_factor: i64::sample(tick + 12),
            min_stability: i64::sample(tick + 13),
            max_stability: i64::sample(tick + 14),
            war_support_offensive_war: i64::sample(tick + 15),
            war_support_defensive_war: i64::sample(tick + 16),
            war_support_tension_impact: i64::sample(tick + 17),
            min_war_support: i64::sample(tick + 18),
            max_war_support: i64::sample(tick + 19),
            interception_war_support_scale_q15: i64::sample(tick + 20),
            interception_bombing_war_support_impact: i64::sample(tick + 21),
            base_max_command_power: i64::sample(tick + 22),
            keys: parts.iter().map(|(_, key)| *key).collect(),
            children: u32::sample(tick),
            language: Vec::<u8>::sample(tick),
            ..Default::default()
        };
        [
            row.stability_factor,
            row.party_popularity_stability_factor,
            row.war_stability_factor,
            row.offensive_war_stability_factor,
            row.defensive_war_stability_factor,
            row.war_support_factor,
            row.stability_weekly,
            row.war_support_weekly,
        ] = std::array::from_fn(|index| parts[index].0);
        row
    }
}
impl Fixture for CountryStabilityLineRow {
    fn fixture(key: u128, tick: usize) -> Self {
        let kind = (tick % (Self::DYNAMIC_MODIFIER_VALUES as usize + 1)) as u8;
        Self {
            country: (key >> 64) as u32,
            line: (key >> 32) as u32,
            key: key as u32,
            kind,
            identity: Vec::<u8>::sample(tick),
            identity_id: u32::sample(tick),
            secondary: i64::sample(tick + 1),
            value: i64::sample(tick + 2),
            text: if kind == Self::UNPAIRED {
                // Nonempty: the unpaired kind falls on ticks divisible by 9.
                Vec::<u8>::sample(tick + 1)
            } else {
                Vec::new()
            },
        }
    }
}
fixture!(ResearchInputRow, |key| {country: (key >> 32) as u32, index: key as u32}, {technology_handle: u32, technology_present: u8, token: Vec<u8>, classifier: u32, level: i32, progress_raw: i64, cost_raw: i64, cost_scale_raw: i64, cost_debug: u8, research_speed_raw: i64, banked_days_raw: i64, applied_days_raw: i64, ahead_reduction_raw: i64, speed_bonus_raw: i64, bonus_count: u32});
fixture!(ResearchAppliedBonusRow, |key| {country: (key >> 64) as u32, index: (key >> 32) as u32, ordinal: key as u32}, {technology_handle: u32, id: i32, resolved: u8, name: Vec<u8>, ahead_reduction_raw: i64, speed_bonus_raw: i64, cost_reduction_raw: Option<i64>});
fixture!(ResearchSlotRow, |key| {country: (key >> 32) as u32, index: key as u32}, {technology_handle: u32, technology_present: u8, research_speed_raw: i64, banked_days_raw: i64, applied_days_raw: i64});
fixture!(TechnologyDefinitionRow, |key| {technology_handle: key as u32}, {database_index: i32, year: i32, max_level: i32, research_cost_raw: i64, id: Vec<u8>});
fixture!(PoliticsRow, |key| {country: key as u32}, {last_election: i32, election_frequency: i32, elections_allowed: u8, capital: i32, ruling_party: Option<u32>, original_capital: i32});
fixture!(PoliticalPartyRow, |key| {country: (key >> 32) as u32, id: key as u32}, {popularity: i64, is_ruling: u8, token: Vec<u8>, tag: u32, name_default: u8, name: Vec<u8>, long_name: Vec<u8>});
fixture!(ActiveIdeaRow, |key| {country: (key >> 32) as u32, ordinal: key as u32}, {token: Vec<u8>});
fixture!(DecisionCountsRow, |key| {country: key as u32}, {taken: u32, re_enable: u32, remove: u32, timed: u32, targeted_timed: u32, targeted: u32});
fixture!(DecisionRow, |key| {country: (key >> 40) as u32, container: (key >> 32) as u8, ordinal: key as u32}, {token: Vec<u8>, days: i32, lifecycle: i32, target_kind: u8, country_target: u32, numeric_target: u32, ignore: u8});
fixture!(DynamicModifierRow, |key| {country: (key >> 32) as u32, ordinal: key as u32}, {scope_country: u32, scope_state: i32, days_remaining: i32, enabled: u8, token: Vec<u8>, values: Vec<i64>});
fixture!(PreferredTacticRow, |key| {country: key as u32}, {tactic_id: i32, token: Vec<u8>});
fixture!(CountryTacticUnlockRow, |key| {country: key as u32}, {unlocked_tactic_ids: Vec<u32>});
fixture!(TechSharingGroupRow, |key| {id: key as u32}, {base_bonus: i64, member_count: u32});
fixture!(TechSharingMemberRow, |key| {group: (key >> 32) as u32, ordinal: key as u32}, {country: u32, bonus: i64});
fixture!(PowerBalanceRow, |key| {ordinal: key as u32}, {value_raw: i64, balance: Vec<u8>, left: Vec<u8>, right: Vec<u8>, trending: Vec<u8>, active_range: Vec<u8>});
fixture!(PowerBalanceMemberRow, |key| {record: (key >> 32) as u32, ordinal: key as u32}, {country: u32, daily_raw: i64, weekly_raw: i64, daily_present: bool, weekly_present: bool, daily_flags: u32, weekly_flags: u32});
fixture!(PowerBalanceRangeRow, |key| {record: (key >> 32) as u32, ordinal: key as u32}, {identity: Vec<u8>});
fixture!(PowerBalanceOverrideRow, |key| {record: (key >> 32) as u32, ordinal: key as u32}, {side: Vec<u8>, gfx: Vec<u8>});
fixture!(ProductionLineRow, |key| {handle: key as u64}, {country: u32, kind: u8, variant: u64, mio: u64, active_factories: i32, requested_factories: i32, cost_raw: i64, speed_raw: i64, produced_raw: i64, priority: i32, amount: i32});
fixture!(StockpileRow, |key| {country: (key >> 32) as u32, ordinal: key as u32}, {variant_handle: u64, amount_raw: i64});
fixture!(DivisionEquipmentRow, |key| {division: (key >> 32) as u64, ordinal: key as u32}, {template: u64, country: u32, variant_handle: u64, amount_raw: i64});
fixture!(VariantDefinitionRow, |key| {id: key as u64}, {equipment_token: u32, creator_link: u32, origin_link: u32, append_variant_name: u8, obsolete: u8, classifier_flags: u64, mission_mask: u32, display_name: Vec<u8>, variant_name: Vec<u8>, explicit_entity: Vec<u8>, archetype_token: u32});
sample_struct!(LocalSubunitRequirement {
    archetype_token: u32,
    amount_raw: i64
});
fixture!(CountryLocalSubunitRow, |key| {country: (key >> 32) as u32, token: key as u32}, {requirements: Vec<LocalSubunitRequirement>});
impl Fixture for AvailableEquipmentVariantRow {
    fn fixture(key: u128, tick: usize) -> Self {
        Self {
            country: (key >> 34) as u32,
            roster_kind: ((key >> 32) & 3) as u8,
            ordinal: key as u32,
            variant: u64::sample(tick),
        }
    }
    /// Roster kind 3 is refused: the largest key has kind 2.
    fn largest_key() -> u128 {
        (u128::from(u32::MAX) << 34) | (2 << 32) | u128::from(u32::MAX)
    }
}
fixture!(VariantModuleRow, |key| {variant: (key >> 32) as u64, source_ordinal: key as u32}, {slot: u32, definition_token: u32, definition_active: u8});
fixture!(VariantStatsRow, |key| {variant: key as u64}, {values: [i64;10]});
fixture!(VariantUpgradeRow, |key| {variant: (key >> 32) as u64, source_ordinal: key as u32}, {definition_token: i32, level: u8});
fixture!(ArmyUpgradeGroupRow, |key| {army: key as u64}, {country: u32, priority: u32});
fixture!(ArmyUpgradeRequestRow, |key| {army: (key >> 32) as u64, ordinal: key as u32}, {country: u32, equipment: u64, request_raw: i64, total_raw: i64});
fixture!(ArmyUpgradeDeliveryRow, |key| {army: (key >> 32) as u64, ordinal: key as u32}, {country: u32, status: u32, progress_raw: Option<i64>, total_progress_raw: i64, allow_zero_entries: u8});
fixture!(ArmyUpgradeProducedRow, |key| {army: (key >> 64) as u64, delivery_ordinal: (key >> 32) as u32, ordinal: key as u32}, {country: u32, equipment: u64, amount_raw: i64});
fixture!(OrdersGroupRow, |key| {handle: key as u64}, {country: u32, is_army_group: u8, motorization_level: u8, theatre: u64, theatre_ordinal: Option<u32>, theater_group: u64, parent: u64, leader: u64, hq_provider: u64});
fixture!(OrdersGroupMemberRow, |key| {group: (key >> 32) as u64, ordinal: key as u32}, {unit: u64});
fixture!(ArmyGroupMemberRow, |key| {group: (key >> 32) as u64, ordinal: key as u32}, {child: u64});
fixture!(IncomingDiplomaticActionRow, |key| {country: (key >> 32) as u32, ordinal: key as u32}, {handle: u64, status: u8, action_type: i32, actor: u32, original_actor: u32, recipient: u32, original_recipient: u32, hour: i32, last_command_hour: i32});
fixture!(ScientistRow, |key| {character: key as u32}, {country: u32, ordinal: u32, trait_count: i32, skill_count: i32});
fixture!(ScientistSkillRow, |key| {character: (key >> 32) as u32, ordinal: key as u32}, {token: u32, level: i32, experience_raw: i64});
fixture!(CountryFuelRow, |key| {country: key as u32}, {current_raw: i64, maximum_raw: i64, gain_raw: i64, cost_raw: i64, gain_from_states_raw: i64, lend_lease_gain_raw: i64, lend_lease_consumption_raw: i64, gain_per_oil_raw: i64, history_index: i32, remaining_hours: i32, ring_gain_raw: i64, ring_consumed_raw: i64, ring_other_raw: i64, ring_requested_0_raw: i64, ring_requested_1_raw: i64, ring_requested_2_raw: i64, ring_received_0_raw: i64, ring_received_1_raw: i64, ring_received_2_raw: i64});
fixture!(CountryFuelConsumerRow, |key| {country: (key >> 32) as u32, index: key as u32}, {priority: i32, requested_raw: i64, received_raw: i64});
fixture!(ResourceDefinitionRow, |key| {id: key as u32}, {token: u32, active: u8, factory_cost: i64});
fixture!(CountryResourceRow, |key| {country: (key >> 32) as u32, resource: key as u32}, {produced: i64, adjustment: i64, imported: i64, used: i64, special_total: i64, production_demand: i64, to_export: i64, base_export: i64, exported: i64});
fixture!(ResourceExchangeRow, |key| {id: key as u64}, {exporter: u32, importer: u32, resource: u32, import_ordinal: u32, export_ordinal: u32, convoys_assigned: i32, convoys_required: i32, efficiency_a: i64, efficiency_b: i64, delivered: i64, required_factories: i32, assigned_factories: i32, no_trade_influence: u8, no_resources: u8});
fixture!(PurchaseContractRow, |key| {id: key as u64}, {seller: u32, buyer: u32, speed: i32, convoys_assigned: i32, convoys_total: i32, route_efficiency_sum_raw: i64, route_days: i32, factory_cic_progress_raw: i64, equipment_transfer_progress_raw: i64, collected_raw: i64, completed_deliveries: i32});
fixture!(ProvinceSupplyRow, |key| {province: key as u32}, {controller_index: u16, capital_country_index: u16, supply_node_level: u8, naval_base_level: u8, flags: u8});
fixture!(HubMotorizationRow, |key| {owner: (key >> 96) as u32, province: (key >> 64) as u32 as i32, node_type: (key >> 32) as u32 as i32, actor: key as u32}, {node_index: u32, map_entry_index: u32, level: u8});
fixture!(DisruptedSupplyRow, |key| {country: (key >> 64) as u32, province: (key >> 32) as u32, node_type: key as u32}, {value: i64});
fixture!(ForeignHomebaseRow, |key| {country: (key >> 32) as u32, ordinal: key as u32}, {province: u32, supply: i64, start: i64, penalty: i64, added_penalty: i64, duration: i64, hours: i64, decay: i64, base: u8, source: Vec<u8>});
fixture!(CountryExperienceRow, |key| {country: key as u32}, {army_raw: i64, army_daily_raw: i64, army_daily_training_raw: i64, navy_raw: i64, navy_daily_raw: i64, navy_daily_training_raw: i64, air_raw: i64, air_daily_raw: i64, armies_for_training_raw: i64});
fixture!(BuildingRow, |key| {location: (key >> 32) as u32, building_type: key as u32}, {ordinal: u32, status_flags: u32, country: u32, level: i16, healthy_level: i16, partial_health: i64, active: bool});
fixture!(StateBaseResourceRow, |key| {state: (key >> 32) as u32, resource: key as u32}, {ordinal: u32, amount: i64});
fixture!(BuildingConstructionRow, |key| {id: key as u64}, {country: u32, queue_ordinal: u32, active_factories: i32, cost: i64, speed: i64, produced: i64, priority: i32, amount: i32, created_hour: i32, building_type: u32, target_flags: u32, target_location: u32, target_state: u32, target_country: u32, repair_levels: i32});
fixture!(RailwayConstructionRow, |key| {id: key as u64}, {country: u32, queue_ordinal: u32, active_factories: i32, cost: i64, speed: i64, produced: i64, priority: i32, amount: i32, created_hour: i32, base_level: i32, target_level: i32, cancelled: u8, current_segment: i32, remaining_cost: i64, route: Vec<u32>});
fixture!(OccupationSettingsRow, |key| {country: key as u32}, {garrison_template: Option<u64>, default_law_present: u8, law: Vec<u8>});
fixture!(StateOccupationLawRow, |key| {controller: (key >> 64) as u32, occupied: (key >> 32) as u32, state: key as u32}, {name: Vec<u8>});
fixture!(StateGarrisonRow, |key| {controller: (key >> 32) as u32, state: key as u32}, {occupied: u32, required_manpower: i32, strength_ratio: i64, garrison_required: i64, serialized_flag: u8});
fixture!(StateAssignedManpowerRow, |key| {controller: (key >> 64) as u32, state: (key >> 32) as u32, ordinal: key as u32}, {occupied: u32, source_country: u32, amount: i32});
fixture!(CountryOccupationReserveManpowerRow, |key| {country: (key >> 32) as u32, ordinal: key as u32}, {source_country: u32, amount: i32});
fixture!(StateResistanceRow, |key| {state: key as u32}, {country: u32, level: i64, speed: i64, compliance: i64, compliance_speed: i64, active: u8});
fixture!(CountryCollaborationRow, |key| {source: (key >> 32) as u32, target: key as u32}, {progress: i64});
fixture!(CharacterRow, |key| {id: key as u32}, {country: u32, name_token_id: i32, roster: u8, ordinal: u32, name: Vec<u8>, token: Vec<u8>, template_present: u8, template_tag: u32, template_file: Vec<u8>, template_line: i32, template_generated: u8, gender: u32});
fixture!(CountryLeaderRoleRow, |key| {character: (key >> 32) as u32, ideology: key as u32 as i32}, {country: u32});
fixture!(AdvisorRoleRow, |key| {character: (key >> 32) as u32, ordinal: key as u32}, {country: u32, key: Vec<u8>, idea: Vec<u8>, slot: Vec<u8>});
fixture!(UnitLeaderRow, |key| {id: (key >> 32) as u32, reference_type: key as u32}, {character_type: u32, character_id: u32, category: i32, experience: i64, kind: u8, skill_status: u8, skill_valid: u8, skill_level: i32, skill_required_experience: i32, skill_category: i32, components: [i32;4], inherited_text_a: Vec<u8>, inherited_text_b: Vec<u8>, owner: u32, captured: u8, visibility_choice: u8, template_visibility: u8});
fixture!(UnitLeaderTraitRow, |key| {leader_reference: (key >> 32) as u64, ordinal: key as u32}, {token_id: i32, definition_ordinal: i32, token: Vec<u8>});
fixture!(UnitLeaderTraitExperienceRow, |key| {leader_reference: (key >> 32) as u64, ordinal: key as u32}, {token_id: i32, definition_ordinal: i32, token: Vec<u8>, experience: i64});
fixture!(ActiveAbilityRow, |key| {country: (key >> 32) as u32, ordinal: key as u32}, {scope_country: u32, token: Vec<u8>, caster: Option<u64>, strategic_region: Option<u32>, division_count: i32, activation_hour: i32, phase: u8, allocate_cost: u8, allocated_command_power: i64, duration_hours: i32, cooldown_hours: i32, target_kind: i32});
fixture!(CapitulationRow, |key| {country: key as u32}, {capitulated: u8, capitulated_hour: i32, last_surrender_hour: i32, faction_join_hour: i32, legitimacy_raw: i64, exile_army_leaders: i32});
fixture!(SubjectRow, |key| {overlord: (key >> 32) as u32, subject: key as u32}, {start_hour: i32, end_hour: i32, cancel: u8, autonomy_ratio: i64, state: Vec<u8>});
fixture!(AutonomyRow, |key| {subject: key as u32}, {score: i64, lower_score: i64, upper_score: i64, last_modified_bits: u32, state: Vec<u8>, previous: Option<Vec<u8>>, next: Option<Vec<u8>>});
fixture!(IntelligenceAgencyRow, |key| {country: key as u32}, {upgrade_progress_raw: i64, defense_raw: i64, active_count: i32, admission_ceiling: i32, unknown_f0: u32, name: Vec<u8>, created: u8});
fixture!(OperativeMembershipRow, |key| {id: (key >> 40) as u64, roster_country: (key >> 8) as u32, roster_kind: key as u8}, {owner_country: u32, detected: u8, assignment: u32, experience_raw: i64, skill: Option<i32>, capturer_country: u32, captured_hour: i32, action_variant: Option<u32>});
fixture!(CryptologyRow, |key| {owner_country: (key >> 32) as u32, target_country: key as u32}, {amount_raw: i64, bonus_days: i32, bonus_active: u8, decrypting: u8});
fixture!(IntelNetworkRow, |key| {owner_country: (key >> 64) as u32, target_country: (key >> 32) as u32, state_id: key as u32 as i32}, {strength_raw: Option<i64>, subnetwork_selector: i32, local_strength_raw: i64});
fixture!(ScriptFlagRow, |key| {scope_kind: (key >> 64) as u8, scope_id: (key >> 32) as u32, token: key as u32}, {set_hour: i32, value: i16, remaining_days: i16});
impl Fixture for GlobalSavedEventTargetRow {
    fn fixture(key: u128, tick: usize) -> Self {
        Self {
            ordinal: key as u32,
            name: format!("target_{tick}").into_bytes(),
            state: tick as i32,
            country: -(tick as i32),
            character: tick as u32,
            live: Self::UNSAVED_NAME | (tick % 2048) as u16,
            character_leader: tick.is_multiple_of(2).then_some(tick as u64),
        }
    }
    fn largest_key() -> u128 {
        u128::from(u32::MAX)
    }
}
fixture!(ScriptVariablesRow, |key| {scope_kind: (key >> 32) as u8, scope_id: key as u32}, {entries: Vec<ScriptVariableEntry>});
fixture!(OperationRow, |key| {id: key as u64}, {owner: u32, target: u32, target_province: Option<u32>, definition_token: u32, preparation_current: i64, preparation_required: i64, flag_40: u8, auto_repeat: u8, terminal: u8, start_hour: i32, duration_days: i32, phase_count: u32, assigned_slots: u32});
fixture!(OperationAssignmentRow, |key| {operation: (key >> 32) as u64, ordinal: key as u32}, {operative: u64, selection: u32});
fixture!(WingEquipmentRow, |key| {wing: (key >> 32) as u64, ordinal: key as u32}, {variant: u64, amount_raw: i64});
fixture!(AirBaseRow, |key| {id: key as u64}, {state: Option<u32>, province: Option<u32>, host: u64, capacity: i32, kind: i32});
fixture!(WingCumulativeRow, |key| {id: key as u64}, {accidents: i32, experience: i64});
fixture!(AirRegionHistoryRow, |key| {country: (key >> 32) as u32, region: key as u32}, {entries: Vec<AirHistoryEntry>});
fixture!(AirRegionDamageRow, |key| {country: (key >> 32) as u32, region: key as u32}, {entries: Vec<AirDamageEntry>});
fixture!(AirAceRow, |key| {id: key as u64}, {country: u32, ordinal: u32, aircraft_type_mask: u32, assigned_wing: u64, alive: u8, handled: u8, kill_type: u32, death: Option<AceDeath>, name: Vec<u8>, surname: Vec<u8>, callsign: Vec<u8>});
fixture!(AirStatisticsRow, |key| {country: (key >> 32) as u32, region: key as u32}, {year_slot: u32, year_window_wrapped: u8, channels: [i64;24]});
fixture!(SunkConvoyRow, |key| {month: (key >> 64) as u32 as i32, killer_country: (key >> 32) as u32, owner_country: key as u32}, {convoys: i32, ordinal: u32});
fixture!(RailCooldownRow, |key| {province: key as u32}, {cooldown_ticks: i32});
fixture!(RailEdgeRow, |key| {from: (key >> 32) as u32, to: key as u32}, {level: i32});
fixture!(LandTheatreRow, |key| {id: key as u64}, {owner: u32, ordinal: u32, volunteers_theatre: i32});
fixture!(LandTheatreGroupRow, |key| {id: key as u64}, {theatre: u64, ordinal: u32, priority: i32, name: Vec<u8>});
fixture!(LandTheatreMemberRow, |key| {theater_group: (key >> 64) as u64, orders_group: key as u64}, {ordinal: u32});
fixture!(AirTheatreRow, |key| {id: key as u64}, {owner: u32, ordinal: u32, name: Vec<u8>});
fixture!(AirTheatreGroupRow, |key| {id: key as u64}, {theatre: u64, ordinal: u32, name: Vec<u8>});
fixture!(AirTheatreMemberRow, |key| {group: (key >> 64) as u64, wing: key as u64}, {ordinal: u32});
fixture!(NavalResultRow, |key| {id: key as u64}, {province: u32, result_hour: i32, retire_hour: i32, category: u32, flag_a: u8, flag_b: u8, subtype: u8, readiness: u8, winning_side: u8, losing_side: u8, province_ordinal: u32});
fixture!(NavalResultMemberRow, |key| {result: (key >> 64) as u64, member: key as u64}, {presence_value: u64, ordinal: u32, side: u8});
fixture!(NavalResultCountryRow, |key| {result: (key >> 40) as u64, list: (key >> 32) as u8, country: key as u32}, {ordinal: u32});
fixture!(SunkShipsRow, |_key| {}, {entries: Vec<SunkShipEntry>});
sample_struct!(RaidCounterOperative {
    handle: u64,
    defense: i64,
    context: i64
});
fixture!(RaidIntelInputRow, |key| {country: key as u32}, {strategic_present: u8, country_defense: i64, crypto_strength: Option<i64>, operatives: Vec<RaidCounterOperative>});
fixture!(RaidAirScoreRow, |key| {country: (key >> 32) as u32, region: key as u32}, {friendly: i64, enemy: i64});
fixture!(RaidTooltipOperandRow, |key| {raid: (key >> 32) as u64, ordinal: key as u32}, {kind: u32, key: u32, raw: i64});
impl Fixture for CharacterRoleTraitRow {
    fn fixture(key: u128, tick: usize) -> Self {
        Self {
            character: (key >> 40) as u32,
            kind: (key >> 32) as u8,
            role: key as u32,
            tokens: vec![u32::sample(tick), 7],
            definitions: if (key >> 32) as u8 == Self::SCIENTIST {
                Vec::new()
            } else {
                vec![u32::sample(tick + 1), 3]
            },
        }
    }
    /// A scientist row (the last kind) keeps role 0.
    fn largest_key() -> u128 {
        (u128::from(u32::MAX) << 40) | (u128::from(Self::SCIENTIST) << 32)
    }
}
impl Fixture for CharacterOperativeRow {
    fn fixture(key: u128, tick: usize) -> Self {
        let resolved = tick % 2 == 1;
        Self {
            character: key as u32,
            reference: u64::sample(tick).max(1),
            resolved,
            category: if resolved { i32::sample(tick) } else { 0 },
            owner: if resolved { u32::sample(tick) } else { 0 },
            skill_level: if resolved {
                Option::<i32>::sample(tick)
            } else {
                None
            },
            trait_definitions: if resolved {
                Vec::<u32>::sample(tick)
            } else {
                Vec::new()
            },
        }
    }
}
fixture!(RaidFacilityRow, |key| {country: (key >> 32) as u32, ordinal: key as u32}, {id: u32, owner: u32, province: u32, active_project: Option<u32>, project_definition: Option<u32>});
fixture!(ProjectProgramRow, |key| {id: key as u32}, {owner: u32, province: u32, active_project: Option<u32>, scientist: u64, unread_rewards: i32, building: Vec<u8>});
fixture!(ProjectRow, |key| {owner: (key >> 32) as u32, local_id: key as u32}, {definition_id: u32, program_reference: u64, started: u8, completed: u8, state: ProjectState});
fixture!(ProjectHistoryRow, |key| {owner: (key >> 64) as u32, project: (key >> 32) as u32, ordinal: key as u32}, {prototype_reward_handle: u32, option_handle: u32, date: i32, context: ProjectContext});
fixture!(ProjectBreakthroughRow, |key| {owner: (key >> 32) as u32, specialization: key as u32}, {points: i32});
fixture!(NukeRow, |key| {country: (key >> 32) as u32, nuke_type: key as u32}, {amount_raw: i64, ready: i32});
fixture!(RaidCountryRow, |key| {canonical_slot: key as u32}, {country: i32, eligible_types: Vec<u32>, used_types: Vec<u32>});
fixture!(RaidRow, |key| {id: (key >> 32) as u32, reference_type: key as u32}, {parent_slot: u32, ordinal: u32, phase: i32, outcome: i32, source: RaidSource, show_for: Vec<u32>, unit: u64, air_wing: u64, travelled: Option<i64>, preparation_hours: i32});
fixture!(RaidTargetRow, |key| {id: (key >> 32) as u32, reference_type: key as u32}, {scope: RaidScope, raid_type: u32, detected: u8});
fixture!(NavalBaseRow, |key| {province: key as u32}, {country: u32, level: i32, healthy_level: i32, countries: Vec<u32>, countries_unreadable: bool});
fixture!(PeaceConferenceRow, |key| {id: key as u64}, {ordinal: u32, current_turn: i32, finalized: u8});
fixture!(WeatherRegionRow, |key| {id: key as u32}, {max_snow: i64, light_rain: u8, heavy_rain: u8, snow: u8, blizzard: u8, sandstorm: u8, arctic_water: u8});
fixture!(WeatherProvinceRow, |key| {id: key as u32}, {snow: i64, permanent_snow: i64, display_snow: u8, display_mud: u8});
fixture!(OperationTokensRow, |key| {owner: (key >> 32) as u32, target: key as u32}, {tokens: Vec<u32>});
fixture!(LexicalTokenRow, |key| {token: key as u32}, {literal: Vec<u8>});
fixture!(HiredAdvisorRow, |key| {country: (key >> 32) as u32, ordinal: key as u32}, {character: u32, idea: Vec<u8>, slot: Vec<u8>});
fixture!(NavalSupremacyRow, |key| {region: (key >> 32) as u32, country: key as u32}, {region_slot: u32, current: i64, base_target: i64, target: i64, individual_ratio: i64, previous: i64, decline_from: i64});
fixture!(RadarCoverageRow, |key| {country: (key >> 32) as u32, ordinal: key as u32}, {state: Option<u32>, provinces: Vec<u32>});
fixture!(SentVolunteerRow, |key| {country: (key >> 32) as u32, ordinal: key as u32}, {division: u64});
fixture!(VolunteerHostRow, |key| {country: (key >> 32) as u32, position: key as u32}, {host: u32, deployed_divisions: i32});
fixture!(TheatreAreaRow, |key| {theatre: (key >> 32) as u64, ordinal: key as u32}, {representative: Option<u32>, provinces: Vec<u32>});
fixture!(StationedUnitRow, |key| {theatre: (key >> 32) as u64, ordinal: key as u32}, {unit: u64});
fixture!(UnitIntelRow, |key| {unit: (key >> 32) as u64, ordinal: key as u32}, {viewer: u32, level: i8});
fixture!(AirBaseRegistrationRow, |key| {base: (key >> 32) as u64, slot: key as u32}, {capacity_raw: i64, wings: Vec<u64>});
fixture!(ProductionLicenseRow, |key| {country: (key >> 32) as u32, ordinal: key as u32}, {variant: u64});
fixture!(ProductionLicenseRecordRow, |key| {owner: (key >> 32) as u32, ordinal: key as u32}, {giver: u32, lended_cic: i32, required_cic: i32, start_date: i32, parent: u64, operand_raw: i64, equipment: Vec<u64>});
fixture!(DeploymentConveyorRow, |key| {handle: key as u64}, {country: u32, ordinal: u32, template: u64, amount: i32, location: u32, priority: u32, role_token: u32, closed: u8, name: Vec<u8>, exile_country: Option<u32>});
fixture!(DeploymentLineRow, |key| {handle: key as u64}, {conveyor: u64, country: u32, ordinal: u32, name_order: i32, amount: i32, training_raw: i64, max_training_raw: i64, max_manpower: i32, allow_zero_entries: u8, equipment_allocations: i32, distributable_priority: i32, halting_reason_present: u8, halting_reason: i32, custom_name: Vec<u8>, custom_flag_a: u8, custom_flag_b: u8});
fixture!(DeploymentManpowerRow, |key| {line: (key >> 33) as u64, side: ((key >> 32) & 1) as u8, ordinal: key as u32}, {source_country: u32, amount: i32});
fixture!(DeploymentEquipmentRow, |key| {line: (key >> 32) as u64, ordinal: key as u32}, {variant_handle: u64, amount_raw: i64});
fixture!(WorldTensionRow, |_key| {}, {total_raw: i64, source_count: i32, country_aggregate_count: i32});
fixture!(ProvinceNameRow, |key| {province: key as u32}, {name: Vec<u8>});
fixture!(StateFlagRow, |key| {state: key as u32}, {demilitarized: u8});
fixture!(ArmyOrderPlanRow, |key| {owner_group: (key >> 32) as u64, instance: key as u32}, {front: u64, front_valid: u8, section: u32, segment_start: i32, segment_end: i32, blitz: u8, withdraw: u8, fallback: u8, virtual_order: u8, has_child: u8, owner_override: u64, assigned_divisions: Vec<u64>, predecessors: Vec<u32>, successors: Vec<u32>, name_prefix: Vec<u8>, name: Vec<u8>, name_first: Vec<u8>, name_second: Vec<u8>, name_unique: Vec<u8>, name_postfix: Vec<u8>});
fixture!(OrdersGroupPlanRow, |key| {group: key as u64}, {red: u32, green: u32, blue: u32, alpha: u32, primary_orders: Vec<u32>, fallback_orders: Vec<u32>, name: Vec<u8>});
impl Fixture for FrontRow {
    fn fixture(key: u128, tick: usize) -> Self {
        Self {
            front: key as u64,
            section_ids: vec![7 + tick as u32, 3],
            section_ends: vec![if tick.is_multiple_of(2) { 0 } else { 2 }, 2],
            province_pairs: vec![tick as u32, tick as u32 + 1],
        }
    }
}
sample_struct!(TrainAllocation {
    variant: u64,
    count: i32
});
/// Empty, short and multi-byte-gap arrays with runs of varied length.
impl Sample for SparseBytes {
    fn sample(tick: usize) -> Self {
        let dense: Vec<u8> = (0..(tick % 3) * 150)
            .map(|index| {
                if index % 5 < 2 && !(index + tick).is_multiple_of(3) {
                    (index % 255 + 1) as u8
                } else {
                    0
                }
            })
            .collect();
        Self::from_dense(&dense).unwrap()
    }
}
fixture!(SupplyNodeRow, |key| {country: (key >> 32) as u32, index: key as u32}, {province: u32, kind: u32, flag: u8, available: i64, consumed: i64, parent: i32, route_from: u16, route_to: u16, break_after: u8, path: Vec<u32>});
// Combat fixture start
sample_struct!(CombatModifierValue {
    id: u32,
    value_a: i64,
    value_b: i64
});
sample_struct!(DivisionCombatModifiers { division: u64, ordinal: u32, entries: Vec<CombatModifierValue> });
sample_struct!(LandCombatCommanderTrait { token_id: i32, definition_ordinal: i32, kind: u32, relevant: u8, token: Vec<u8> });
sample_struct!(LandCombatCommanderLearningTrait { token_id: i32, definition_ordinal: i32, required_experience: i32, experience: i64, token: Vec<u8> });
sample_struct!(LandCombatCommanderPortraitModifier {
    source: u8,
    id: u32,
    value: i64,
    metadata_flags: u32,
    category_mask: u32,
    feature_selector: u32
});
sample_struct!(LandCombatCommanderPortraitTooltip { source: u8, bucket: u32, key: u32, text: Vec<u8> });
sample_struct!(CombatCommanderSkillDefinition {
    present: u8,
    valid: u8,
    level: i32,
    required_experience: i32,
    category: i32
});
sample_struct!(CombatCommanderPortraitEntry { role: u32, size: u32, is_set: u8, kind: u32, text: Vec<u8> });
sample_struct!(CombatCommanderPresentation { character: u64, character_state: u8, character_country: u32, character_gender: u32, character_role_leader: u64, character_fallback_leader: u64, character_role_source: u8, fallback_text: [Vec<u8>;3], portraits: Vec<CombatCommanderPortraitEntry>, role_traits: Vec<LandCombatCommanderTrait> });
sample_struct!(NavalCommanderInputs { commander: u64, country: u32, category: i32, experience: i64, skills: [i32; 5], skill_definitions: [CombatCommanderSkillDefinition;5], next_skill: CombatCommanderSkillDefinition, name: Vec<u8>, description: Vec<u8>, presentation: CombatCommanderPresentation, traits: Vec<LandCombatCommanderTrait>, learning_traits: Vec<LandCombatCommanderLearningTrait>, learning_threshold: i64, modifiers: Vec<LandCombatCommanderPortraitModifier>, modifier_feature_flags: u32, modifier_callback_kind: u8, tooltips: Vec<LandCombatCommanderPortraitTooltip> });
sample_struct!(LandCombatDivisionCommander { skill: i32, name_token: Vec<u8>, traits: Vec<LandCombatCommanderTrait> });
sample_struct!(LandCombatSideDetails { committed: Vec<u64>, reserves: Vec<u64>, tactic_id: i32, tactic_kind: u8, tactic_counter_id: u32, tactic_token: Vec<u8>, air_counts: Vec<i32>, anti_air_attack: i64, air_kills: u32, damage: [i64; 6], organisation_loss: [i64; 24], organisation_loss_count: u32, commander: u64, commander_skill: i32, commander_category: i32, commander_country: u32, commander_experience: i64, commander_name_token: Vec<u8>, commander_description_token: Vec<u8>, commander_formation: u64, presentation: CombatCommanderPresentation, traits: Vec<LandCombatCommanderTrait>, timed_traits: Vec<LandCombatCommanderTimedTrait>, learning_traits: Vec<LandCombatCommanderLearningTrait> });
sample_struct!(LandCombatExileInputs { origin_type: i32, country: i32, country_fallback: i32, location_cores: Vec<u32>, occupiers: Vec<u32> });
sample_struct!(LandCombatCommanderTimedTrait {
    token_id: i32,
    definition_ordinal: i32,
    remaining_days: i32
});
sample_struct!(LandCombatTacticCountryWeight {
    country: u32,
    tactic_id: i32,
    modifier_key: u32,
    metadata_flags: u32,
    metadata_gate: i32,
    country_factor_raw: i64,
    accumulator_raw: i64,
    accumulator_entry_present: u8
});
sample_struct!(LandCombatTacticInputs {
    preferred_tactic_ids: [[i32; 3]; 2],
    country_weights: [LandCombatTacticCountryWeight; 2],
    country_weight_present: [u8; 2]
});
sample_struct!(LandDivisionModifier {
    raw: i64,
    present: u8
});
sample_struct!(LandDivisionWidthTerm {
    subunit: u32,
    count: i32,
    width: i64
});
sample_struct!(LandDivisionContext {
    template: u64,
    leader_modifier_kind: u8,
    country_modifiers: [LandDivisionModifier; 3],
    leader_modifiers: [LandDivisionModifier; 3],
    modifier_metadata: [[u32; 2]; 3],
    feature_flags: u32,
    eligibility_callback: u8,
    base_speed: i64
});
sample_struct!(LandReinforcementModifier {
    scope: u8,
    key: u32,
    raw: i64,
    present: u8,
    metadata_flags: u32,
    feature_selector: u32
});
sample_struct!(LandReinforcementSupplyTerm {
    subunit: u32,
    count: i32,
    special: u8
});
sample_struct!(LandReinforcementAcclimatization {
    weight: i64,
    active: u8
});
sample_struct!(LandReinforcementTransportTerm {
    country: u32,
    regional_value: i64,
    own_intel: i64,
    enemy_intel: i64
});
sample_struct!(LandReinforcementInputs {
    scalar_values: [i64; 32],
    flags: [u8; 12],
    ids: [u32; 8]
});
sample_struct!(NavalCombatSummaryModifier {
    raw: i64,
    present: u8
});
sample_struct!(NavalCombatSummaryInputs { recent_join_penalty: i64, escort_bonus: i64, countries: Vec<u32>, secondary_countries: Vec<u32>, modifiers: [[NavalCombatSummaryModifier; 5]; 5], modifier_metadata: [[u32; 2]; 5], feature_flags: u32, eligibility_callback: u8 });
sample_struct!(NavalCombatAirEfficiencyModifier {
    raw: i64,
    present: u8
});
sample_struct!(NavalCombatAirEfficiency {
    source: u8,
    pool_type_flags: u64,
    mission: i32,
    container_present: u8,
    base_crowding: i64,
    night: i64,
    modifiers: [NavalCombatAirEfficiencyModifier; 3],
    modifier_metadata: [[u32; 2]; 4],
    feature_flags: u32,
    eligibility_callback: u8
});
sample_struct!(NavalCombatAirWingKill {
    wing: u64,
    index: u32,
    category: u64,
    count: i32
});
sample_struct!(NavalCombatPlane { source: u8, group_index: u32, index: u32, carrier_slot: i32, carrier_member_present: u8, carrier_state: u32, air_base: u64, wings: [Vec<u64>;2], initial_planes: i32, current_planes: i32, casualties: i32, time_duration: i32, country: u32, date: i32, flags: [u8; 2], variant: u64, naval_hits: Vec<NavalCombatHit>, air_hits: [Vec<NavalCombatAirHit>;2], kills: Vec<NavalCombatAirWingKill>, efficiency: NavalCombatAirEfficiency });
sample_struct!(NavalCombatSideDetails { commander: u64, engaged: u8, retreating: u8, capital_screening: i64, carrier_screening: i64, positioning: i64, anti_air: i64, planes: Vec<NavalCombatPlane>, summary: NavalCombatSummaryInputs, commander_inputs: Option<NavalCommanderInputs>, units: Vec<u64> });
sample_struct!(NavalCombatDamage {
    source: u64,
    country: u32,
    damage: i64
});
sample_struct!(NavalCombatAirHit {
    variant: u64,
    count: i32,
    country: u32
});
sample_struct!(NavalCombatHit { target_slot: u32, target_name: Vec<u8>, convoy: u8, damage: i64, denominator: i64, last_hit: u8 });
sample_struct!(NavalCombatLastTarget {
    present: u8,
    is_convoy: u8,
    target_group_member_count: i32
});
sample_struct!(NavalShipModifierTerm {
    key: u32,
    raw: i64,
    metadata_flags: u32,
    feature_selector: u32
});
sample_struct!(NavalShipModifierSource { scope: u8, index: u32, selector: u32, reference: u64, weights: [i64; 2], dynamic_keys: [u32; 2], terms: Vec<NavalShipModifierTerm> });
sample_struct!(NavalShipWeaponContext { task_force: u64, country: u32, commander: u64, ship_class: u32, subunit: u32, weapon_bases: [i64; 5], weather_factor: i64, fuel: [i64; 2], night: i64, formation_flags: u64, detection_base: i64, air_base: u64, air_base_maximum: i32, air_detection_base: i64, terrain_name: Vec<u8>, sources: Vec<NavalShipModifierSource> });
sample_struct!(NavalResultShip { member: u64, owner: u32, subunit_index: i32, formation_type: i32, build_cost_ic: i64, strength_ratio: i64, pride: u8, is_convoy: u8, name: Vec<u8>, type_name: Vec<u8>, sprite: Vec<u8>, sunk_by: Vec<u8>, variant: u64, convoy_client: u64, convoy_index: i32, naval_hits: Vec<NavalCombatHit>, air_hits: [Vec<NavalCombatAirHit>;2] });
sample_struct!(NavalResultPlane { variant: u64, variant_summary_inputs: [i64; 2], initial_planes: i32, current_planes: i32, lost_planes: i32, country: u32, name: Vec<u8>, naval_hits: Vec<NavalCombatHit>, air_hits: [Vec<NavalCombatAirHit>;2] });
sample_struct!(NavalResultSideDetails { leader: u64, hq_leader: u64, hq_influences: Vec<u64>, ships: Vec<NavalResultShip>, planes: Vec<NavalResultPlane>, hq_modifiers: Vec<NavalShipModifierSource>, commander_context: Option<NavalCommanderInputs>, hq_commander_context: Option<NavalCommanderInputs> });
fixture!(LandCombatDetailsRow, |key| { combat: key as u64 }, { province: u32, phase: i32, width_factor: i64, attacker_bonus: i64, defender_bonus: i64, stored_terrain_valid: u8, stored_terrain_name: Vec<u8>, sides: [LandCombatSideDetails;2], tactic_inputs: LandCombatTacticInputs });
fixture!(LandCombatDivisionRow, |key| { combat: (key >> 33) as u64, side: ((key >> 32) & 1) as u8, ordinal: key as u32 }, { role: u8, province: u32, division: u64, name_type: i32, name_order: i32, name_ordered: u8, name_programmatic: u8, name_custom: Vec<u8>, name_group: Vec<u8>, strength_combat_factor: i64, fuel_inputs: [i64; 2], stat_values: [i64; 5], armour: i64, piercing: i64, air_attack: i64, reconnaissance: i64, initiative: i64, commander: u64, scales: [i64; 4], context: LandDivisionContext, width_terms: Vec<LandDivisionWidthTerm>, commander_inputs: Option<LandCombatDivisionCommander>, reinforcement: LandReinforcementInputs, reinforcement_modifiers: Vec<LandReinforcementModifier>, reinforcement_supply: Vec<LandReinforcementSupplyTerm>, reinforcement_acclimatization: Vec<LandReinforcementAcclimatization>, reinforcement_transport: Vec<LandReinforcementTransportTerm>, direction_group: i32, exile: Option<LandCombatExileInputs>, categories: u64 });
fixture!(LandCombatModifierRow, |key| { combat: (key >> 1) as u64, side: (key & 1) as u8 }, { border_values: Option<[i64;3]>, divisions: Vec<DivisionCombatModifiers> });
impl Fixture for LandCombatSideRow {
    fn fixture(key: u128, tick: usize) -> Self {
        // Secondary sets hold distinct positive tags only.
        let positive = |offset: usize| {
            (0..(tick + offset) % 3)
                .map(|index| (tick + offset + index) as u32 + 1)
                .collect()
        };
        Self {
            combat: key as u64,
            indicator: u8::sample(tick),
            flags: <[u32; 2]>::sample(tick),
            attacker_primary: Vec::<u32>::sample(tick),
            attacker_secondary: positive(1),
            defender_primary: Vec::<u32>::sample(tick + 1),
            defender_secondary: positive(2),
        }
    }
}
fixture!(NavalCombatDetailsRow, |key| { combat: key as u64 }, { province: u32, day: i32, duration: i32, progress: i64, sides: [NavalCombatSideDetails;2] });
fixture!(NavalCombatShipRow, |key| { combat: (key >> 33) as u64, side: ((key >> 32) & 1) as u8, battle_slot: key as u32 }, { ship: u64, owner: u32, state: u32, member_source: u8, group_index: u32, member_index: u32, convoy: u64, subunit_index: i32, formation_type: i32, build_cost_ic: i64, cached_strength_ratio: i64, pride: u8, is_convoy: u8, name: Vec<u8>, type_name: Vec<u8>, sprite: Vec<u8>, sunk_by: Vec<u8>, variant: u64, arrival_hours: i32, received_criticals: i32, evacuation_state: i32, detection_state: i32, escape_progress: i64, last_target: NavalCombatLastTarget, received_damage: [i64; 6], live_ship: u8, live_convoy: u8, strength: i64, organisation: i64, base_organisation: i64, maximum_hit_points: i64, formation_flags: u64, weapon_context: NavalShipWeaponContext, damage_entries: Vec<NavalCombatDamage>, air_hits: [Vec<NavalCombatAirHit>;2], naval_hits: Vec<NavalCombatHit> });
fixture!(NavalCombatResultDetailsRow, |key| { result: key as u64 }, { sides: [NavalResultSideDetails;2] });
// Combat fixture end
fixture!(TrainInventoryRow, |key| {country: key as u32}, {node_count: u32, allocations: Vec<TrainAllocation>, controller_access: SparseBytes});
fixture!(CountryAppearanceRow, |key| {country: key as u32}, {red: u32, green: u32, blue: u32, alpha: u32, cosmetic_tag: Vec<u8>, name_tag: Vec<u8>, leader_ideology: Vec<u8>, overlord: i32, graphical_culture: Vec<u8>, leader_character: Option<u32>, leader_traits: Vec<u32>, old_name: Option<Vec<u8>>, old_adjective: Option<Vec<u8>>, old_definite_name: Option<Vec<u8>>, ui_red: u32, ui_green: u32, ui_blue: u32, ui_alpha: u32});
fixture!(ArmyIconInputRow, |key| {division: key as u64}, {separate_counter: u8, alternate_template: u64});
fixture!(ProvinceUnitOrderRow,|key| {province:key as u32},{mixed:Vec<u64>,naval:Vec<u64>});
sample_struct!(CharacterPortrait {category:u32,size:u32,is_set:u8,kind:u32,identity:Vec<u8>});
fixture!(CharacterPortraitRow,|key| {character:key as u32},{portraits:Vec<CharacterPortrait>});
fixture!(LeaderPortraitRow,|key| {leader:key as u64},{character:u64,portrait_c0:Vec<u8>,portrait_e0:Vec<u8>,portrait_100:Vec<u8>});
fixture!(TaskForceCounterInputRow, |key| {task_force: key as u64}, {is_in_regions: u8, running_away: u8, engagement_rule: i32, mission_region_count: i32, bombardment_region: u32, repair_parent: u64, detached_activity: i32, repair_last_mission: i32, received_supply_ratio: i64, attached_headquarters: i32});
fixture!(TaskForceMapMotionRow, |key| {task_force: key as u64}, {target: u32, combat_role: u8, path: Vec<u32>, alternate_path: Vec<u32>});
fixture!(FloatingHarborDeploymentRow, |key| {country: (key >> 32) as u32, ordinal: key as u32}, {province: i32, status: u8});
fixture!(FacilityMapFlagRow, |key| {province: key as u32}, {flag: u8});
impl Fixture for NavalProvinceCombatRow {
    fn fixture(key: u128, tick: usize) -> Self {
        let combats = Vec::<u64>::sample(tick);
        let kinds = (0..combats.len())
            .map(|index| ((tick + index) % (Self::NAVAL as usize + 1)) as u8)
            .collect();
        Self {
            province: key as u32,
            combats,
            kinds,
        }
    }
}
fixture!(NavalCombatTransferRow, |key| {combat: key as u64}, {transfers: Vec<u64>});
fixture!(NavalTransferUnitRow, |key| {transfer: key as u64}, {units: Vec<u64>});
fixture!(TaskForceMissionRow, |key| {task_force: key as u64}, {owner: u32, current_region: u32, regions: Vec<u32>});
fixture!(NavalCombatAirBaseWingHandlesRow, |key| { base: key as u64 }, { wings: Vec<u64> });
fixture!(RailwayGunCounterInputRow, |key| {gun: key as u64}, {roster_owner: u32, expeditionary_owner: u32, logical_owner: u32, province: u32, naval_transfer: u8, transfer_action_status: u8, exiled: u8, retreat: u8, strategic_redeployment: u8, reference_entry_count: i32, assignee_group: u64, subunit_token: u32, current_strength: i64, maximum_strength: i64, received_supply_ratio: i64, railway_building_level: Option<i16>, target: u32, combat_role: u8, combat_province: u32, combat_count: u32, attacking: u8, ordinary_path: Vec<u32>, alternate_path: Vec<u32>});
fixture!(FleetCounterInputRow, |key| {fleet: key as u64}, {red: u32, green: u32, blue: u32, alpha: u32, mission_region_count: i32, task_forces: Vec<u64>});
fixture!(CountryCounterInputRow, |key| {country: key as u32}, {pride_of_the_fleet: u64, owned_state_count: i32, controlled_state_count: i32, first_division: u64});
fixture!(OrdersGroupCounterInputRow, |key| {group: key as u64}, {withdrawing: u8});
fixture!(LeaderPortraitPathRow, |key| {leader: key as u64}, {country: u32, tag_reference: u32, tag: Vec<u8>});
impl Fixture for ShipModelRow {
    fn fixture(key: u128, tick: usize) -> Self {
        Self {
            ship: key as u64,
            variant: u64::sample(tick),
            hull_sprite: Vec::<u8>::sample(tick),
            fallback_sprite: Vec::<u8>::sample(tick + 1),
            model_number: i32::sample(tick),
            formation_flags: u64::sample(tick),
            counter: Some(ShipCounterInputs {
                roster_ordinal: tick as u32,
                maximum_strength: i64::sample(tick),
                anti_air_base: i64::sample(tick + 1),
                ..Default::default()
            }),
        }
    }
}
fixture!(LocalPlayerRow, |_key| {}, {country: u32, observer_country: u32});
fixture!(SimulationFeatureRow, |_key| {}, {dlc_mask: u32, human_ai: u8});
fixture!(RaidPlayerAllowedTypeRow, |key| {country: key as u32}, {allowed: Vec<u32>});
impl Fixture for ModifierDefinitionRow {
    fn fixture(key: u128, tick: usize) -> Self {
        Self {
            index: key as u32,
            token: u32::sample(tick),
            name_key: Vec::<u8>::sample(tick + 1),
            postfix_key: Vec::<u8>::sample(tick + 2),
            argument: Vec::<u8>::sample(tick + 3),
            flags: u32::sample(tick + 4),
            category_mask: u32::sample(tick + 5),
            feature_selector: u32::sample(tick + 6),
            precision: i32::sample(tick + 7),
            // The codec admits only the two callback states.
            eligibility: (tick % 2) as u8,
        }
    }
}
fixture!(CountryVisibilityRow, |key| {country: key as u32}, {unit_modifier: i64});
fixture!(StateVisibilityRow, |key| {state: key as u32}, {country: u32, network_modifier: i64});
fixture!(RelationLinkRow, |key| {country: (key >> 32) as u32, target_index: key as u32}, {links: u8});
fixture!(CountryWarRow, |key| {country: key as u32}, {can_occupy_non_war: u8, enemies_read: u8, enemies: Vec<u32>});
fixture!(NavalRegionAccessRow, |key| {country: (key >> 32) as u32, region: key as u32}, {mode: u8});
fixture!(BorderConflictRow, |key| {province: key as u32}, {groups: Vec<u64>});
fixture!(RaidTargetStatusRow, |key| {ordinal: key as u32}, {valid: u8, scope: RaidScope, detected: Vec<u8>});
fixture!(RaidPathDistanceRow, |key| {source: (key >> 32) as u32, target: key as u32}, {distance: i64, ok: u8});
fixture!(RaidCooldownRow, |key| {owner_slot: (key >> 32) as u32, ordinal: key as u32}, {raid_type: u32, scope: RaidScope, days: i32});
fixture!(StateResourceInputRow, |key| {state: key as u32}, {gate: u8, global_factor: i64, origin: u8, infrastructure_level: Option<i32>, infrastructure_modifier: i64, supply_route: u8});
fixture!(StateResourceRow, |key| {state: (key >> 32) as u32, resource: key as u32}, {pool: i64, additive: i64, factor: i64, cost: i64});
fixture!(StateDynamicModifierRow, |key| {state: (key >> 32) as u32, ordinal: key as u32}, {token: Vec<u8>, scope_country: u32, scope_state: i32});
fixture!(CountryBuildingResourceRow, |key| {country: (key >> 32) as u32, ordinal: key as u32}, {entry: u32, building: u32, count: u32, resource: u32, amount: i64});
fixture!(RaidCountryInputRow, |key| {canonical_slot: key as u32}, {air_range_factor: Option<i64>, naval_commando_range_factor: Option<i64>, autonomous: Option<u32>, convoy_reservations: Vec<u32>});
fixture!(CountryFactoryCountsRow, |key| {country: key as u32}, {military: i32, naval: i32, civilian: i32});
fixture!(CountryCommandPowerRow, |key| {canonical_slot: key as u32}, {allocated_command_power_raw: i64, max_command_power: Option<i64>, max_command_power_factor: Option<i64>, ace_effectiveness_factor: Option<i64>, air_ace_bonuses_factor: Option<i64>});
fixture!(DivisionRetreatBanRow, |key| {division: key as u64}, {owner: u32, bans: u8});
fixture!(StateCoreRow, |key| {state: key as u32}, {cores: Vec<u32>});
fixture!(InvasionPreparationRow, |key| {owner_group: (key >> 32) as u64, instance: key as u32}, {progress: i64, target: u32, country_term: i64, relation_term: Option<i64>, relation_bonus: Option<i64>, leader_term: Option<i64>, leader_applies: u8, days_term: i64, unit_owner: u32, zero_war_present: u8, zero_war_retired: u8, zero_co_belligerent: u8, co_belligerents: Vec<u32>});
fixture!(CountryCharacterRow, |key| {country: (key >> 32) as u32, ordinal: key as u32}, {character: Option<u32>});
fixture!(PartyLeaderRow, |key| {country: (key >> 64) as u32, party: (key >> 32) as u32, ordinal: key as u32}, {character: Option<u32>, leader_id: i32});
fixture!(CountryCivilWarRow, |key| {country: (key >> 32) as u32, ordinal: key as u32}, {target: u32});
fixture!(AdvisorTokenRow, |key| {ordinal: key as u32}, {token: Vec<u8>});
fixture!(FactionAdvisorRow, |key| {faction: (key >> 32) as u64, ordinal: key as u32}, {token: Vec<u8>});
fixture!(RaidEquipmentRow, |key| {id: (key >> 32) as u32, reference_type: key as u32}, {phase: i32, end_hour: Option<i32>, variants: Vec<u64>, amounts: Vec<i64>, nukes: i32, archetype_tokens: Vec<u32>, archetype_amounts: Vec<i64>});
fixture!(RaidSpeedInputRow, |key| {id: (key >> 32) as u32, reference_type: key as u32}, {phase: i32, outcome: i32, end_hour: Option<i32>, wing_max_speed: Option<i64>, army_max_speed: Option<i64>, transport_speeds: Vec<i64>, transport_from_country: u8, transport_fallback: u8, ace_present: u8, ace_alive: u8, ace_speed: i64, ace_type_speed: i64, ace_wing_planes: Option<i32>, ace_owner: Option<u32>, wing_country: Option<u32>});
fixture!(StockpileArchetypeRow, |key| {canonical_slot: (key >> 32) as u32, archetype: key as u32}, {quantity_raw: i64});
fixture!(RaidAirSiteRow, |key| {canonical_slot: (key >> 32) as u32, ordinal: key as u32}, {list: u8, base: u64, container_capacity_raw: Option<i64>});
fixture!(AceRangeInputRow, |key| {canonical_slot: (key >> 64) as u32, ace: key as u64}, {wing: u64, alive: u8, range: i64, type_range: i64});
fixture!(SliceReadRow, |key| {table: (key >> 32) as u32, owner: key as u32}, {sample_hour: i32});
fixture!(NavalMissionTaskForceRow, |key| {task_force: key as u64}, {raiding_factor: i64, local_coordination: Option<i64>, local_raiding: Option<i64>, weather_coordination: Option<i64>, weather_raiding: Option<i64>, leader: u64, headquarters: Vec<u64>, range_fleet: u64, range_begin: i32, range_end: i32, convoys: i32, range_base: i64, fuel: i64, fuel_requested: i64, underway: u8, replenishment: i32, replenishment_limit: i32, local_range: Option<i64>, local_range_factor: Option<i64>, weather_range: Option<i64>, weather_range_factor: Option<i64>, mission_owner: u32});
impl Fixture for NavalMissionLedgerRow {
    fn fixture(key: u128, tick: usize) -> Self {
        let ring = !tick.is_multiple_of(2);
        let mut task_forces = Vec::<u64>::sample(tick);
        if task_forces.is_empty() {
            task_forces.push(u64::sample(tick));
        }
        Self {
            owner: (key >> 32) as u32,
            region: key as u32,
            task_forces,
            ring,
            positive: if ring { i32::sample(tick) } else { 0 },
            total: if ring { i32::sample(tick + 1) } else { 0 },
        }
    }
}
fixture!(NavalMissionCountryRow, |key| {country: key as u32}, {coordination: Option<i64>, raiding: Option<i64>, escort: Option<i64>, weather_reduction: Option<i64>, range: Option<i64>, range_factor: Option<i64>, underway: Option<i64>, hubs: Vec<u32>});
impl Fixture for NavalMissionCommanderRow {
    fn fixture(key: u128, tick: usize) -> Self {
        Self {
            country: (key >> 72) as u32,
            reference: (key >> 8) as u64,
            source: key as u8,
            coordination: Option::sample(tick),
            raiding: Option::sample(tick + 1),
            range: Option::sample(tick + 2),
            range_factor: Option::sample(tick + 3),
        }
    }
    /// Sources above 4 are refused: the largest key has source 4.
    fn largest_key() -> u128 {
        (u128::from(u32::MAX) << 72) | (u128::from(u64::MAX) << 8) | 4
    }
}
fixture!(FactionTheatreRow, |key| {faction: (key >> 32) as u64, ordinal: key as u32}, {icon: Vec<u8>, regions: Vec<u32>, country_count: u32, hidden_for: Vec<u32>});
fixture!(AirRegionalMissionRow, |key| {wing: key as u64}, {ordinal: u32});
fixture!(AirCountryRelationRow, |key| {country: (key >> 34) as u32, kind: ((key >> 32) & 3) as u8, ordinal: key as u32}, {other: u32});
fixture!(AirReferenceWitnessRow, |key| {handle: key as u64}, {base_kind: Option<i32>, state: Option<u32>, province: Option<u32>, host: Option<u64>, host_owner: Option<u32>, host_without_task_force: bool});
fixture!(AirMissionInputRow, |key| {wing: key as u64}, {coverage_raw: i64, reassignment_penalty_raw: i64, last_target_province: i32, reached_count: i32, reached_provinces: Vec<u32>, definition_map_flags: u64});
fixture!(AirSortieBaseCountryRow, |key| {base: (key >> 32) as u64, country: key as u32}, {capacity_raw: i64, supply_raw: i64, fuel_demand_raw: Option<i64>, fuel_allocated_raw: Option<i64>});
fixture!(AirSortieModifierRow, |key| {scope: (key >> 64) as u8, subject: (key >> 32) as u32, region: key as u32}, {raw: i64});
fixture!(AirSortieSpecialBaseRow, |key| {base: key as u64}, {rocket_slots: Option<i32>, carrier_stance: Option<i32>, leader_modifier_raw: i64, task_force_combats: Option<i32>, task_force_region: Option<u32>});
fixture!(AirSortieModifierMetadataRow, |_key| {}, {flags: u32, feature_selector: u32, callback_kind: u8});
fixture!(NavalMineLevelRow, |key| {country: (key >> 32) as u32, region: key as u32}, {raw_mines: i64});
fixture!(AirNavalPatrolCombatRow, |key| {country: (key >> 32) as u32, ordinal: key as u32}, {province: u32, region: u32});
fixture!(AirNavalPatrolHistoryRow, |key| {country: (key >> 64) as u32, region: (key >> 32) as u32, index: key as u32}, {enemy_ships_sunk_raw: i64, our_fighters_lost_raw: i64});
fixture!(DivisionMovementRow, |key| {division: key as u64}, {source: u32, target: u32, numerator: i64, retreat: u8, strategic_redeployment: u8, naval_transfer: u8, combat_count: u32, path_total: u32, path: Vec<u32>, alternate_total: u32, alternate_path: Vec<u32>, combat_role: u8, combat_province: u32, map_entity: u8});
sample_struct!(DivisionAcclimatization {
    category: u32,
    weight: i64
});
fixture!(DivisionStatusRow, |key| {division: key as u64}, {maximum_hit_points: i64, maximum_organisation: i64, entrenchment: i64, entrenchment_cap: i64, supply_ratio: i64, attrition_value: i64, attrition_threshold: i64, attrition: u8, exercising: u8, has_order: u8, invalid_orders: u8, strength_factor: i64, model_number: i32, acclimatization_maximum: Option<i64>, acclimatization: Vec<DivisionAcclimatization>, move_flags: u8});
impl Fixture for DivisionEtaInputsRow {
    fn fixture(key: u128, tick: usize) -> Self {
        // Leg vectors must stay aligned, so they cannot be sampled apart.
        let legs = |offset: usize| {
            (0..tick % 3)
                .map(|i| i64::sample(tick + offset + i))
                .collect()
        };
        Self {
            division: key as u64,
            sample_hour: i32::sample(tick),
            speed_modifier: i64::sample(tick),
            speed_base: i64::sample(tick + 1),
            speed_org: Option::<i64>::sample(tick),
            speed_fuel: Option::<i64>::sample(tick + 1),
            current_speed: i64::sample(tick + 1),
            // Distinct positive indices, one past a byte.
            terrain_speed: (0..tick % 3)
                .map(|i| TerrainSpeed {
                    terrain: [1, 256, 1581][i],
                    modifier: i64::sample(tick + i),
                })
                .collect(),
            fort_penalty: i64::sample(tick),
            snow_penalty: i64::sample(tick + 1),
            river_adjustment: i64::sample(tick + 2),
            river_leader: i64::sample(tick),
            class: u32::sample(tick),
            rail_country_modifier: Option::<i64>::sample(tick),
            rail_state_modifier: Option::<i64>::sample(tick + 1),
            rail_friendly_state_modifier: Option::<i64>::sample(tick + 2),
            leg_river_combined: legs(1),
            leg_transport_speed: if tick.is_multiple_of(2) {
                Vec::new()
            } else {
                legs(2)
            },
            leg_rail_speed: if tick.is_multiple_of(3) {
                Vec::new()
            } else {
                legs(3)
            },
        }
    }
}
sample_struct!(TemplateSubunit {
    token: u32,
    local: u8,
    count: i32
});
fixture!(DivisionTemplateRow, |key| {template: key as u64}, {owner: u32, name: Vec<u8>, unit_type: i32});
fixture!(DivisionTemplateIconRow, |key| {template: key as u64}, {priority_unit: u32, priority_unit_name: Vec<u8>, custom_icon: i32, custom_icon_nato: u8, representative: u32, representative_local: u8, subunits: Vec<TemplateSubunit>, country: i32, foreign_country: i32, model_category: Option<u64>, model_entity: Vec<u8>, excluded_variants: Vec<u64>, model_unreadable: u8});
fixture!(LandCombatProgressRow, |key| {combat: key as u64}, {attacker_progress: i64, defender_progress: i64, direction_attacker: u64, direction_province: u32});
fixture!(ThreatSourceRow, |key| {ordinal: key as u32}, {country: u32, second_country: u32, current_raw: i64, final_raw: i64, daily_raw: i64, date: i32, reason: Vec<u8>});
fixture!(ThreatCountryAggregateRow, |key| {country_slot: key as u32}, {total_raw: i64});
fixture!(RelationOpinionRow, |key| {owner_country: (key >> 32) as u32, target_country: key as u32}, {opinion: i32});
fixture!(RelationTupleRow, |key| {owner_country: (key >> 80) as u32, target_country: (key >> 48) as u32, type_id: (key >> 16) as u32, ordinal: key as u16}, {first_country: u32, second_country: u32, start_hour: i32, end_hour: i32, cancel: u8, retired: u8});
fixture!(CountryIntelRow, |key| {owner_country: (key >> 32) as u32, target_index: key as u32}, {army: i64, air: i64, civilian: i64, navy: i64});

fn maximum_key<T: RowCodec>() -> u128 {
    if T::KEY_BITS == 128 {
        u128::MAX
    } else {
        (1_u128 << T::KEY_BITS) - 1
    }
}
fn rows<T: Fixture>(tick: usize) -> Vec<T> {
    let mut rows = vec![T::fixture(0, tick)];
    if T::KEY_BITS != 0 {
        rows.push(T::fixture(1, tick + 1));
        if tick % 17 < 5 {
            rows.push(T::fixture(T::largest_key(), tick + 2));
        }
    }
    rows
}
pub(crate) fn sample_extended_table(table: TableId, tick: usize) -> Option<TableRows> {
    Some(match table {
        TableId::Focus => TableRows::Focus(rows::<FocusRow>(tick)),
        TableId::CompletedFocuses => TableRows::CompletedFocuses(rows::<CompletedFocusRow>(tick)),
        TableId::ResearchTechnologies => {
            TableRows::ResearchTechnologies(rows::<ResearchTechnologyRow>(tick))
        }
        TableId::ResearchSlots => TableRows::ResearchSlots(rows::<ResearchSlotRow>(tick)),
        TableId::TechnologyDefinitions => {
            TableRows::TechnologyDefinitions(rows::<TechnologyDefinitionRow>(tick))
        }
        TableId::Politics => TableRows::Politics(rows::<PoliticsRow>(tick)),
        TableId::PoliticalParties => TableRows::PoliticalParties(rows::<PoliticalPartyRow>(tick)),
        TableId::ActiveIdeas => TableRows::ActiveIdeas(rows::<ActiveIdeaRow>(tick)),
        TableId::DecisionCounts => TableRows::DecisionCounts(rows::<DecisionCountsRow>(tick)),
        TableId::Decisions => TableRows::Decisions(rows::<DecisionRow>(tick)),
        TableId::DynamicModifiers => TableRows::DynamicModifiers(rows::<DynamicModifierRow>(tick)),
        TableId::PreferredTactics => TableRows::PreferredTactics(rows::<PreferredTacticRow>(tick)),
        TableId::CountryTacticUnlocks => {
            TableRows::CountryTacticUnlocks(rows::<CountryTacticUnlockRow>(tick))
        }
        TableId::TechSharingGroups => {
            TableRows::TechSharingGroups(rows::<TechSharingGroupRow>(tick))
        }
        TableId::TechSharingMembers => {
            TableRows::TechSharingMembers(rows::<TechSharingMemberRow>(tick))
        }
        TableId::PowerBalances => TableRows::PowerBalances(rows::<PowerBalanceRow>(tick)),
        TableId::PowerBalanceMembers => {
            TableRows::PowerBalanceMembers(rows::<PowerBalanceMemberRow>(tick))
        }
        TableId::PowerBalanceRanges => {
            TableRows::PowerBalanceRanges(rows::<PowerBalanceRangeRow>(tick))
        }
        TableId::PowerBalanceOverrides => {
            TableRows::PowerBalanceOverrides(rows::<PowerBalanceOverrideRow>(tick))
        }
        TableId::ProductionLines => TableRows::ProductionLines(rows::<ProductionLineRow>(tick)),
        TableId::Stockpiles => TableRows::Stockpiles(rows::<StockpileRow>(tick)),
        TableId::DivisionEquipment => {
            TableRows::DivisionEquipment(rows::<DivisionEquipmentRow>(tick))
        }
        TableId::VariantDefinitions => {
            TableRows::VariantDefinitions(rows::<VariantDefinitionRow>(tick))
        }
        TableId::VariantModules => TableRows::VariantModules(rows::<VariantModuleRow>(tick)),
        TableId::VariantStats => TableRows::VariantStats(rows::<VariantStatsRow>(tick)),
        TableId::VariantUpgrades => TableRows::VariantUpgrades(rows::<VariantUpgradeRow>(tick)),
        TableId::ArmyUpgradeGroups => {
            TableRows::ArmyUpgradeGroups(rows::<ArmyUpgradeGroupRow>(tick))
        }
        TableId::ArmyUpgradeRequests => {
            TableRows::ArmyUpgradeRequests(rows::<ArmyUpgradeRequestRow>(tick))
        }
        TableId::ArmyUpgradeDeliveries => {
            TableRows::ArmyUpgradeDeliveries(rows::<ArmyUpgradeDeliveryRow>(tick))
        }
        TableId::ArmyUpgradeProduced => {
            TableRows::ArmyUpgradeProduced(rows::<ArmyUpgradeProducedRow>(tick))
        }
        TableId::CountryFuel => TableRows::CountryFuel(rows::<CountryFuelRow>(tick)),
        TableId::CountryFuelConsumers => {
            TableRows::CountryFuelConsumers(rows::<CountryFuelConsumerRow>(tick))
        }
        TableId::ResourceDefinitions => {
            TableRows::ResourceDefinitions(rows::<ResourceDefinitionRow>(tick))
        }
        TableId::CountryResources => TableRows::CountryResources(rows::<CountryResourceRow>(tick)),
        TableId::ResourceExchanges => {
            TableRows::ResourceExchanges(rows::<ResourceExchangeRow>(tick))
        }
        TableId::PurchaseContracts => {
            TableRows::PurchaseContracts(rows::<PurchaseContractRow>(tick))
        }
        TableId::ProvinceSupply => TableRows::ProvinceSupply(rows::<ProvinceSupplyRow>(tick)),
        TableId::HubMotorization => TableRows::HubMotorization(rows::<HubMotorizationRow>(tick)),
        TableId::DisruptedSupply => TableRows::DisruptedSupply(rows::<DisruptedSupplyRow>(tick)),
        TableId::ForeignHomebases => TableRows::ForeignHomebases(rows::<ForeignHomebaseRow>(tick)),
        TableId::CountryExperience => {
            TableRows::CountryExperience(rows::<CountryExperienceRow>(tick))
        }
        TableId::StateBuildings => TableRows::StateBuildings(rows::<BuildingRow>(tick)),
        TableId::ProvinceBuildings => TableRows::ProvinceBuildings(rows::<BuildingRow>(tick)),
        TableId::StateBaseResources => {
            TableRows::StateBaseResources(rows::<StateBaseResourceRow>(tick))
        }
        TableId::BuildingConstruction => {
            TableRows::BuildingConstruction(rows::<BuildingConstructionRow>(tick))
        }
        TableId::RailwayConstruction => {
            TableRows::RailwayConstruction(rows::<RailwayConstructionRow>(tick))
        }
        TableId::OccupationSettings => {
            TableRows::OccupationSettings(rows::<OccupationSettingsRow>(tick))
        }
        TableId::StateOccupationLaws => {
            TableRows::StateOccupationLaws(rows::<StateOccupationLawRow>(tick))
        }
        TableId::StateGarrisons => TableRows::StateGarrisons(rows::<StateGarrisonRow>(tick)),
        TableId::StateAssignedManpower => {
            TableRows::StateAssignedManpower(rows::<StateAssignedManpowerRow>(tick))
        }
        TableId::CountryOccupationReserveManpower => TableRows::CountryOccupationReserveManpower(
            rows::<CountryOccupationReserveManpowerRow>(tick),
        ),
        TableId::StateResistance => TableRows::StateResistance(rows::<StateResistanceRow>(tick)),
        TableId::CountryCollaboration => {
            TableRows::CountryCollaboration(rows::<CountryCollaborationRow>(tick))
        }
        TableId::Characters => TableRows::Characters(rows::<CharacterRow>(tick)),
        TableId::CountryLeaderRoles => {
            TableRows::CountryLeaderRoles(rows::<CountryLeaderRoleRow>(tick))
        }
        TableId::AdvisorRoles => TableRows::AdvisorRoles(rows::<AdvisorRoleRow>(tick)),
        TableId::UnitLeaders => TableRows::UnitLeaders(rows::<UnitLeaderRow>(tick)),
        TableId::UnitLeaderTraits => TableRows::UnitLeaderTraits(rows::<UnitLeaderTraitRow>(tick)),
        TableId::UnitLeaderTraitExperience => {
            TableRows::UnitLeaderTraitExperience(rows::<UnitLeaderTraitExperienceRow>(tick))
        }
        TableId::ActiveAbilities => TableRows::ActiveAbilities(rows::<ActiveAbilityRow>(tick)),
        TableId::Capitulation => TableRows::Capitulation(rows::<CapitulationRow>(tick)),
        TableId::Subjects => TableRows::Subjects(rows::<SubjectRow>(tick)),
        TableId::Autonomy => TableRows::Autonomy(rows::<AutonomyRow>(tick)),
        TableId::IntelligenceAgencies => {
            TableRows::IntelligenceAgencies(rows::<IntelligenceAgencyRow>(tick))
        }
        TableId::OperativeMemberships => {
            TableRows::OperativeMemberships(rows::<OperativeMembershipRow>(tick))
        }
        TableId::Cryptology => TableRows::Cryptology(rows::<CryptologyRow>(tick)),
        TableId::IntelNetworks => TableRows::IntelNetworks(rows::<IntelNetworkRow>(tick)),
        TableId::ScriptFlags => TableRows::ScriptFlags(rows::<ScriptFlagRow>(tick)),
        TableId::GlobalSavedEventTargets => {
            TableRows::GlobalSavedEventTargets(rows::<GlobalSavedEventTargetRow>(tick))
        }
        TableId::ScriptVariables => TableRows::ScriptVariables(rows::<ScriptVariablesRow>(tick)),
        TableId::Operations => TableRows::Operations(rows::<OperationRow>(tick)),
        TableId::OperationAssignments => {
            TableRows::OperationAssignments(rows::<OperationAssignmentRow>(tick))
        }
        TableId::WingEquipment => TableRows::WingEquipment(rows::<WingEquipmentRow>(tick)),
        TableId::AirBases => TableRows::AirBases(rows::<AirBaseRow>(tick)),
        TableId::WingCumulative => TableRows::WingCumulative(rows::<WingCumulativeRow>(tick)),
        TableId::AirRegionHistory => TableRows::AirRegionHistory(rows::<AirRegionHistoryRow>(tick)),
        TableId::AirRegionDamage => TableRows::AirRegionDamage(rows::<AirRegionDamageRow>(tick)),
        TableId::AirAces => TableRows::AirAces(rows::<AirAceRow>(tick)),
        TableId::AirStatistics => TableRows::AirStatistics(rows::<AirStatisticsRow>(tick)),
        TableId::SunkConvoys => TableRows::SunkConvoys(rows::<SunkConvoyRow>(tick)),
        TableId::RailCooldowns => TableRows::RailCooldowns(rows::<RailCooldownRow>(tick)),
        TableId::RailEdges => TableRows::RailEdges(rows::<RailEdgeRow>(tick)),
        TableId::LandTheatres => TableRows::LandTheatres(rows::<LandTheatreRow>(tick)),
        TableId::LandTheatreGroups => {
            TableRows::LandTheatreGroups(rows::<LandTheatreGroupRow>(tick))
        }
        TableId::LandTheatreMembers => {
            TableRows::LandTheatreMembers(rows::<LandTheatreMemberRow>(tick))
        }
        TableId::AirTheatres => TableRows::AirTheatres(rows::<AirTheatreRow>(tick)),
        TableId::AirTheatreGroups => TableRows::AirTheatreGroups(rows::<AirTheatreGroupRow>(tick)),
        TableId::AirTheatreMembers => {
            TableRows::AirTheatreMembers(rows::<AirTheatreMemberRow>(tick))
        }
        TableId::NavalResults => TableRows::NavalResults(rows::<NavalResultRow>(tick)),
        TableId::NavalResultMembers => {
            TableRows::NavalResultMembers(rows::<NavalResultMemberRow>(tick))
        }
        TableId::NavalResultCountries => {
            TableRows::NavalResultCountries(rows::<NavalResultCountryRow>(tick))
        }
        TableId::SunkShips => TableRows::SunkShips(rows::<SunkShipsRow>(tick)),
        TableId::RaidTooltipOperands => {
            TableRows::RaidTooltipOperands(rows::<RaidTooltipOperandRow>(tick))
        }
        TableId::CountryRetiredCharacters => {
            TableRows::CountryRetiredCharacters(rows::<CountryCharacterRow>(tick))
        }
        TableId::CharacterRoleTraits => {
            TableRows::CharacterRoleTraits(rows::<CharacterRoleTraitRow>(tick))
        }
        TableId::CharacterOperatives => {
            TableRows::CharacterOperatives(rows::<CharacterOperativeRow>(tick))
        }
        TableId::RaidIntelInputs => TableRows::RaidIntelInputs(rows::<RaidIntelInputRow>(tick)),
        TableId::RaidAirScores => TableRows::RaidAirScores(rows::<RaidAirScoreRow>(tick)),
        TableId::RaidFacilities => TableRows::RaidFacilities(rows::<RaidFacilityRow>(tick)),
        TableId::ProjectPrograms => TableRows::ProjectPrograms(rows::<ProjectProgramRow>(tick)),
        TableId::Projects => TableRows::Projects(rows::<ProjectRow>(tick)),
        TableId::ProjectHistory => TableRows::ProjectHistory(rows::<ProjectHistoryRow>(tick)),
        TableId::ProjectBreakthroughs => {
            TableRows::ProjectBreakthroughs(rows::<ProjectBreakthroughRow>(tick))
        }
        TableId::Nukes => TableRows::Nukes(rows::<NukeRow>(tick)),
        TableId::RaidCountries => TableRows::RaidCountries(rows::<RaidCountryRow>(tick)),
        TableId::Raids => TableRows::Raids(rows::<RaidRow>(tick)),
        TableId::RaidTargets => TableRows::RaidTargets(rows::<RaidTargetRow>(tick)),
        TableId::ResearchModifiers => {
            TableRows::ResearchModifiers(rows::<ResearchModifierRow>(tick))
        }
        TableId::ResearchInputs => TableRows::ResearchInputs(rows::<ResearchInputRow>(tick)),
        TableId::ResearchAppliedBonuses => {
            TableRows::ResearchAppliedBonuses(rows::<ResearchAppliedBonusRow>(tick))
        }
        TableId::NavalBases => TableRows::NavalBases(rows::<NavalBaseRow>(tick)),
        TableId::PeaceConferences => TableRows::PeaceConferences(rows::<PeaceConferenceRow>(tick)),
        TableId::WeatherRegions => TableRows::WeatherRegions(rows::<WeatherRegionRow>(tick)),
        TableId::WeatherProvinces => TableRows::WeatherProvinces(rows::<WeatherProvinceRow>(tick)),
        TableId::OperationTokens => TableRows::OperationTokens(rows::<OperationTokensRow>(tick)),
        TableId::LexicalTokens => TableRows::LexicalTokens(rows::<LexicalTokenRow>(tick)),
        TableId::NavalSupremacy => TableRows::NavalSupremacy(rows::<NavalSupremacyRow>(tick)),
        TableId::RadarCoverage => TableRows::RadarCoverage(rows::<RadarCoverageRow>(tick)),
        TableId::SentVolunteers => TableRows::SentVolunteers(rows::<SentVolunteerRow>(tick)),
        TableId::VolunteerHosts => TableRows::VolunteerHosts(rows::<VolunteerHostRow>(tick)),
        TableId::TheatreAreas => TableRows::TheatreAreas(rows::<TheatreAreaRow>(tick)),
        TableId::StationedUnits => TableRows::StationedUnits(rows::<StationedUnitRow>(tick)),
        TableId::UnitIntel => TableRows::UnitIntel(rows::<UnitIntelRow>(tick)),
        TableId::AirBaseRegistrations => {
            TableRows::AirBaseRegistrations(rows::<AirBaseRegistrationRow>(tick))
        }
        TableId::ProductionLicenses => {
            TableRows::ProductionLicenses(rows::<ProductionLicenseRow>(tick))
        }
        TableId::ProductionLicenseRecords => {
            TableRows::ProductionLicenseRecords(rows::<ProductionLicenseRecordRow>(tick))
        }
        TableId::DeploymentConveyors => {
            TableRows::DeploymentConveyors(rows::<DeploymentConveyorRow>(tick))
        }
        TableId::DeploymentLines => TableRows::DeploymentLines(rows::<DeploymentLineRow>(tick)),
        TableId::DeploymentManpower => {
            TableRows::DeploymentManpower(rows::<DeploymentManpowerRow>(tick))
        }
        TableId::DeploymentEquipment => {
            TableRows::DeploymentEquipment(rows::<DeploymentEquipmentRow>(tick))
        }
        TableId::WorldTension => TableRows::WorldTension(rows::<WorldTensionRow>(tick)),
        TableId::ProvinceNames => TableRows::ProvinceNames(rows::<ProvinceNameRow>(tick)),
        TableId::StateFlags => TableRows::StateFlags(rows::<StateFlagRow>(tick)),
        TableId::ArmyOrderPlans => TableRows::ArmyOrderPlans(rows::<ArmyOrderPlanRow>(tick)),
        TableId::OrdersGroupPlans => TableRows::OrdersGroupPlans(rows::<OrdersGroupPlanRow>(tick)),
        TableId::Fronts => TableRows::Fronts(rows::<FrontRow>(tick)),
        TableId::CountryAppearance => {
            TableRows::CountryAppearance(rows::<CountryAppearanceRow>(tick))
        }
        TableId::ArmyIconInputs => TableRows::ArmyIconInputs(rows::<ArmyIconInputRow>(tick)),
        TableId::ProvinceUnitOrder => {
            TableRows::ProvinceUnitOrder(rows::<ProvinceUnitOrderRow>(tick))
        }
        TableId::CharacterPortraits => {
            TableRows::CharacterPortraits(rows::<CharacterPortraitRow>(tick))
        }
        TableId::LeaderPortraits => TableRows::LeaderPortraits(rows::<LeaderPortraitRow>(tick)),
        TableId::TaskForceCounterInputs => {
            TableRows::TaskForceCounterInputs(rows::<TaskForceCounterInputRow>(tick))
        }
        TableId::RailwayGunCounterInputs => {
            TableRows::RailwayGunCounterInputs(rows::<RailwayGunCounterInputRow>(tick))
        }
        TableId::TaskForceMapMotion => {
            TableRows::TaskForceMapMotion(rows::<TaskForceMapMotionRow>(tick))
        }
        TableId::FloatingHarborDeployments => {
            TableRows::FloatingHarborDeployments(rows::<FloatingHarborDeploymentRow>(tick))
        }
        TableId::FacilityMapFlags => TableRows::FacilityMapFlags(rows::<FacilityMapFlagRow>(tick)),
        TableId::NavalProvinceCombats => {
            TableRows::NavalProvinceCombats(rows::<NavalProvinceCombatRow>(tick))
        }
        TableId::NavalCombatTransfers => {
            TableRows::NavalCombatTransfers(rows::<NavalCombatTransferRow>(tick))
        }
        TableId::NavalTransferUnits => {
            TableRows::NavalTransferUnits(rows::<NavalTransferUnitRow>(tick))
        }
        TableId::TaskForceMissions => {
            TableRows::TaskForceMissions(rows::<TaskForceMissionRow>(tick))
        }
        TableId::NavalCombatAirBaseWingHandles => {
            TableRows::NavalCombatAirBaseWingHandles(rows::<NavalCombatAirBaseWingHandlesRow>(tick))
        }
        TableId::FleetCounterInputs => {
            TableRows::FleetCounterInputs(rows::<FleetCounterInputRow>(tick))
        }
        TableId::CountryCounterInputs => {
            TableRows::CountryCounterInputs(rows::<CountryCounterInputRow>(tick))
        }
        TableId::OrdersGroupCounterInputs => {
            TableRows::OrdersGroupCounterInputs(rows::<OrdersGroupCounterInputRow>(tick))
        }
        TableId::LeaderPortraitPaths => {
            TableRows::LeaderPortraitPaths(rows::<LeaderPortraitPathRow>(tick))
        }
        TableId::ShipModels => TableRows::ShipModels(rows::<ShipModelRow>(tick)),
        TableId::SupplyNodes => TableRows::SupplyNodes(rows::<SupplyNodeRow>(tick)),
        TableId::LandCombatDetails => {
            TableRows::LandCombatDetails(rows::<LandCombatDetailsRow>(tick))
        }
        TableId::LandCombatDivisions => {
            TableRows::LandCombatDivisions(rows::<LandCombatDivisionRow>(tick))
        }
        TableId::LandCombatModifiers => {
            TableRows::LandCombatModifiers(rows::<LandCombatModifierRow>(tick))
        }
        TableId::NavalCombatDetails => {
            TableRows::NavalCombatDetails(rows::<NavalCombatDetailsRow>(tick))
        }
        TableId::NavalCombatShips => TableRows::NavalCombatShips(rows::<NavalCombatShipRow>(tick)),
        TableId::NavalCombatResultDetails => {
            TableRows::NavalCombatResultDetails(rows::<NavalCombatResultDetailsRow>(tick))
        }
        TableId::TrainInventory => TableRows::TrainInventory(rows::<TrainInventoryRow>(tick)),
        TableId::LocalPlayer => TableRows::LocalPlayer(rows::<LocalPlayerRow>(tick)),
        TableId::SimulationFeatures => {
            TableRows::SimulationFeatures(rows::<SimulationFeatureRow>(tick))
        }
        TableId::CountryVisibility => {
            TableRows::CountryVisibility(rows::<CountryVisibilityRow>(tick))
        }
        TableId::StateVisibility => TableRows::StateVisibility(rows::<StateVisibilityRow>(tick)),
        TableId::RelationLinks => TableRows::RelationLinks(rows::<RelationLinkRow>(tick)),
        TableId::SliceReads => TableRows::SliceReads(rows::<SliceReadRow>(tick)),
        TableId::NavalMissionTaskForces => {
            TableRows::NavalMissionTaskForces(rows::<NavalMissionTaskForceRow>(tick))
        }
        TableId::NavalMissionLedgers => {
            TableRows::NavalMissionLedgers(rows::<NavalMissionLedgerRow>(tick))
        }
        TableId::NavalMissionCountries => {
            TableRows::NavalMissionCountries(rows::<NavalMissionCountryRow>(tick))
        }
        TableId::NavalMissionCommanders => {
            TableRows::NavalMissionCommanders(rows::<NavalMissionCommanderRow>(tick))
        }
        TableId::FactionTheatres => TableRows::FactionTheatres(rows::<FactionTheatreRow>(tick)),
        TableId::AirRegionalMissions => {
            TableRows::AirRegionalMissions(rows::<AirRegionalMissionRow>(tick))
        }
        TableId::AirCountryRelations => {
            TableRows::AirCountryRelations(rows::<AirCountryRelationRow>(tick))
        }
        TableId::AirReferenceWitnesses => {
            TableRows::AirReferenceWitnesses(rows::<AirReferenceWitnessRow>(tick))
        }
        TableId::NavalMineLevels => TableRows::NavalMineLevels(rows::<NavalMineLevelRow>(tick)),
        TableId::AirNavalPatrolCombats => {
            TableRows::AirNavalPatrolCombats(rows::<AirNavalPatrolCombatRow>(tick))
        }
        TableId::AirNavalPatrolHistories => {
            TableRows::AirNavalPatrolHistories(rows::<AirNavalPatrolHistoryRow>(tick))
        }
        TableId::AirMissionInputs => TableRows::AirMissionInputs(rows::<AirMissionInputRow>(tick)),
        TableId::AirSortieBaseCountries => {
            TableRows::AirSortieBaseCountries(rows::<AirSortieBaseCountryRow>(tick))
        }
        TableId::AirSortieModifiers => {
            TableRows::AirSortieModifiers(rows::<AirSortieModifierRow>(tick))
        }
        TableId::AirSortieSpecialBases => {
            TableRows::AirSortieSpecialBases(rows::<AirSortieSpecialBaseRow>(tick))
        }
        TableId::AirSortieModifierMetadata => {
            TableRows::AirSortieModifierMetadata(rows::<AirSortieModifierMetadataRow>(tick))
        }
        TableId::RaidCooldowns => TableRows::RaidCooldowns(rows::<RaidCooldownRow>(tick)),
        TableId::RaidPathDistances => {
            TableRows::RaidPathDistances(rows::<RaidPathDistanceRow>(tick))
        }
        TableId::StateResourceInputs => {
            TableRows::StateResourceInputs(rows::<StateResourceInputRow>(tick))
        }
        TableId::StateResources => TableRows::StateResources(rows::<StateResourceRow>(tick)),
        TableId::StateDynamicModifiers => {
            TableRows::StateDynamicModifiers(rows::<StateDynamicModifierRow>(tick))
        }
        TableId::CountryBuildingResources => {
            TableRows::CountryBuildingResources(rows::<CountryBuildingResourceRow>(tick))
        }
        TableId::RaidCountryInputs => {
            TableRows::RaidCountryInputs(rows::<RaidCountryInputRow>(tick))
        }
        TableId::CountryFactoryCounts => {
            TableRows::CountryFactoryCounts(rows::<CountryFactoryCountsRow>(tick))
        }
        TableId::CountryCommandPower => {
            TableRows::CountryCommandPower(rows::<CountryCommandPowerRow>(tick))
        }
        TableId::DivisionRetreatBans => {
            TableRows::DivisionRetreatBans(rows::<DivisionRetreatBanRow>(tick))
        }
        TableId::StateCores => TableRows::StateCores(rows::<StateCoreRow>(tick)),
        TableId::RaidEquipment => TableRows::RaidEquipment(rows::<RaidEquipmentRow>(tick)),
        TableId::InvasionPreparations => {
            TableRows::InvasionPreparations(rows::<InvasionPreparationRow>(tick))
        }
        TableId::CountryCharacters => {
            TableRows::CountryCharacters(rows::<CountryCharacterRow>(tick))
        }
        TableId::PartyLeaders => TableRows::PartyLeaders(rows::<PartyLeaderRow>(tick)),
        TableId::CountryCivilWars => TableRows::CountryCivilWars(rows::<CountryCivilWarRow>(tick)),
        TableId::AdvisorTokens => TableRows::AdvisorTokens(rows::<AdvisorTokenRow>(tick)),
        TableId::FactionAdvisors => TableRows::FactionAdvisors(rows::<FactionAdvisorRow>(tick)),
        TableId::RaidSpeedInputs => TableRows::RaidSpeedInputs(rows::<RaidSpeedInputRow>(tick)),
        TableId::StockpileArchetypes => {
            TableRows::StockpileArchetypes(rows::<StockpileArchetypeRow>(tick))
        }
        TableId::RaidAirSites => TableRows::RaidAirSites(rows::<RaidAirSiteRow>(tick)),
        TableId::AceRangeInputs => TableRows::AceRangeInputs(rows::<AceRangeInputRow>(tick)),
        TableId::CountryWars => TableRows::CountryWars(rows::<CountryWarRow>(tick)),
        TableId::NavalRegionAccess => {
            TableRows::NavalRegionAccess(rows::<NavalRegionAccessRow>(tick))
        }
        TableId::BorderConflicts => TableRows::BorderConflicts(rows::<BorderConflictRow>(tick)),
        TableId::RaidPlayerAllowedTypes => {
            TableRows::RaidPlayerAllowedTypes(rows::<RaidPlayerAllowedTypeRow>(tick))
        }
        TableId::RaidTargetStatuses => {
            TableRows::RaidTargetStatuses(rows::<RaidTargetStatusRow>(tick))
        }
        TableId::CountryLocalSubunits => {
            TableRows::CountryLocalSubunits(rows::<CountryLocalSubunitRow>(tick))
        }
        TableId::AvailableEquipmentVariants => {
            TableRows::AvailableEquipmentVariants(rows::<AvailableEquipmentVariantRow>(tick))
        }
        TableId::NewDivisionEquipment => {
            TableRows::NewDivisionEquipment(rows::<DivisionEquipmentRow>(tick))
        }
        TableId::ModifierDefinitions => {
            TableRows::ModifierDefinitions(rows::<ModifierDefinitionRow>(tick))
        }
        TableId::HiredAdvisors => TableRows::HiredAdvisors(rows::<HiredAdvisorRow>(tick)),
        TableId::CountryStabilityInputs => {
            TableRows::CountryStabilityInputs(rows::<CountryStabilityInputRow>(tick))
        }
        TableId::CountryStabilityLines => {
            TableRows::CountryStabilityLines(rows::<CountryStabilityLineRow>(tick))
        }
        TableId::DivisionMovement => TableRows::DivisionMovement(rows::<DivisionMovementRow>(tick)),
        TableId::DivisionStatus => TableRows::DivisionStatus(rows::<DivisionStatusRow>(tick)),
        TableId::DivisionEtaInputs => {
            TableRows::DivisionEtaInputs(rows::<DivisionEtaInputsRow>(tick))
        }
        TableId::DivisionTemplates => {
            TableRows::DivisionTemplates(rows::<DivisionTemplateRow>(tick))
        }
        TableId::DivisionTemplateIcons => {
            TableRows::DivisionTemplateIcons(rows::<DivisionTemplateIconRow>(tick))
        }
        TableId::LandCombatProgress => {
            TableRows::LandCombatProgress(rows::<LandCombatProgressRow>(tick))
        }
        TableId::LandCombatSides => TableRows::LandCombatSides(rows::<LandCombatSideRow>(tick)),
        TableId::ThreatSources => TableRows::ThreatSources(rows::<ThreatSourceRow>(tick)),
        TableId::ThreatCountryAggregates => {
            TableRows::ThreatCountryAggregates(rows::<ThreatCountryAggregateRow>(tick))
        }
        TableId::OrdersGroups => TableRows::OrdersGroups(rows::<OrdersGroupRow>(tick)),
        TableId::OrdersGroupMembers => {
            TableRows::OrdersGroupMembers(rows::<OrdersGroupMemberRow>(tick))
        }
        TableId::ArmyGroupMembers => TableRows::ArmyGroupMembers(rows::<ArmyGroupMemberRow>(tick)),
        TableId::IncomingDiplomaticActions => {
            TableRows::IncomingDiplomaticActions(rows::<IncomingDiplomaticActionRow>(tick))
        }
        TableId::Scientists => TableRows::Scientists(rows::<ScientistRow>(tick)),
        TableId::ScientistSkills => TableRows::ScientistSkills(rows::<ScientistSkillRow>(tick)),
        TableId::RelationOpinions => TableRows::RelationOpinions(rows::<RelationOpinionRow>(tick)),
        TableId::RelationTuples => TableRows::RelationTuples(rows::<RelationTupleRow>(tick)),
        TableId::CountryIntel => TableRows::CountryIntel(rows::<CountryIntelRow>(tick)),
        _ => return None,
    })
}
fn check_table<T: Fixture>(table: TableId) {
    assert_eq!(table.key_bits(), T::KEY_BITS);
    for key in [0, T::largest_key()] {
        let mut previous = T::fixture(key, 0);
        for tick in 1..=12 {
            let next = T::fixture(key, tick);
            assert_eq!(next.key(), key, "{table:?} key");
            let mut bytes = Vec::new();
            encode_row(&next, 0, &mut bytes).unwrap();
            let mut input = Decoder::new(&bytes);
            assert_eq!(decode_row::<T>(0, &mut input).unwrap(), next, "{table:?}");
            input.finish().unwrap();
            for prefix in 0..bytes.len() {
                assert!(
                    decode_row::<T>(0, &mut Decoder::new(&bytes[..prefix])).is_err(),
                    "{table:?} prefix {prefix}"
                );
            }
            let full = next.delta(None).unwrap().unwrap();
            assert_eq!(full.mask, table.full_mask(), "{table:?}");
            assert_eq!(full.apply::<T>(None).unwrap(), next, "{table:?} insert");
            if let Some(delta) = next.delta(Some(&previous)).unwrap() {
                let mut bytes = Vec::new();
                encode_delta(table, &delta, 0, &mut bytes).unwrap();
                let mut input = Decoder::new(&bytes);
                let decoded = decode_delta(table, 0, &mut input).unwrap();
                input.finish().unwrap();
                assert_eq!(
                    decoded.apply(Some(previous.clone())).unwrap(),
                    next,
                    "{table:?} delta"
                );
                let mut trailing = decoded;
                trailing.values.push(0);
                assert!(trailing.apply(Some(previous.clone())).is_err());
                assert_eq!(
                    previous
                        .delta(Some(&next))
                        .unwrap()
                        .unwrap()
                        .apply(Some(next.clone()))
                        .unwrap(),
                    previous
                );
            } else {
                assert_eq!(previous, next);
            }
            assert!(next.delta(Some(&next)).unwrap().is_none());
            previous = next;
        }
    }
}

/// Character input rows refuse an unknown role kind, a scientist role
/// key, an empty trait list, a zero reference and leader operands on an
/// unresolved reference, in a snapshot and after a delta.
#[test]
fn character_input_rows_refuse_contradictions() {
    let role = CharacterRoleTraitRow {
        character: 1,
        kind: CharacterRoleTraitRow::ADVISOR,
        role: 4,
        tokens: vec![9],
        definitions: vec![2],
    };
    let operative = CharacterOperativeRow {
        character: 1,
        reference: 5,
        resolved: true,
        owner: 9,
        ..Default::default()
    };
    assert!(
        TableRows::CharacterRoleTraits(vec![role.clone()])
            .validate_keys()
            .is_ok()
    );
    assert!(
        TableRows::CharacterOperatives(vec![operative.clone()])
            .validate_keys()
            .is_ok()
    );
    for bad in [
        CharacterRoleTraitRow {
            kind: 3,
            ..role.clone()
        },
        CharacterRoleTraitRow {
            kind: CharacterRoleTraitRow::SCIENTIST,
            ..role.clone()
        },
        CharacterRoleTraitRow {
            tokens: Vec::new(),
            definitions: Vec::new(),
            ..role.clone()
        },
        CharacterRoleTraitRow {
            definitions: Vec::new(),
            ..role.clone()
        },
    ] {
        assert!(encode_row(&bad, 0, &mut Vec::new()).is_err(), "{bad:?}");
        // A snapshot decoded from these bytes fails validation.
        let mut bytes = Vec::new();
        bad.encode_fields(&mut bytes).unwrap();
        let decoded =
            CharacterRoleTraitRow::decode_fields(bad.key(), &mut Decoder::new(&bytes)).unwrap();
        assert!(
            TableRows::CharacterRoleTraits(vec![decoded])
                .validate_keys()
                .is_err()
        );
    }
    for bad in [
        CharacterOperativeRow {
            reference: 0,
            ..operative.clone()
        },
        CharacterOperativeRow {
            resolved: false,
            ..operative.clone()
        },
        CharacterOperativeRow {
            resolved: false,
            owner: 0,
            skill_level: Some(1),
            ..operative.clone()
        },
    ] {
        assert!(encode_row(&bad, 0, &mut Vec::new()).is_err(), "{bad:?}");
        let mut bytes = Vec::new();
        bad.encode_fields(&mut bytes).unwrap();
        let decoded =
            CharacterOperativeRow::decode_fields(bad.key(), &mut Decoder::new(&bytes)).unwrap();
        assert!(
            TableRows::CharacterOperatives(vec![decoded])
                .validate_keys()
                .is_err()
        );
    }
    // A valid delta (an owner of 9) applied to an unresolved baseline
    // gives an unresolved row with an owner, which validation refuses.
    let unresolved = CharacterOperativeRow {
        character: 1,
        reference: 5,
        ..Default::default()
    };
    let delta = operative
        .delta(Some(&CharacterOperativeRow {
            owner: 0,
            ..operative.clone()
        }))
        .unwrap()
        .unwrap();
    let mut bytes = Vec::new();
    encode_delta(TableId::CharacterOperatives, &delta, 0, &mut bytes).unwrap();
    let decoded = decode_delta(TableId::CharacterOperatives, 0, &mut Decoder::new(&bytes)).unwrap();
    let applied = decoded.apply(Some(unresolved)).unwrap();
    assert_eq!(applied.owner, 9);
    assert!(
        TableRows::CharacterOperatives(vec![applied])
            .validate_keys()
            .is_err()
    );
    // And a delta onto a contradictory baseline is refused at once.
    let bad = CharacterOperativeRow {
        reference: 0,
        ..operative.clone()
    };
    assert!(decoded.apply(Some(bad)).is_err());
}

#[test]
fn eta_terrain_entries_are_distinct_database_slots_of_any_width() {
    let entry = |terrain| TerrainSpeed {
        terrain,
        modifier: -10_000,
    };
    let row = |terrain_speed| DivisionEtaInputsRow {
        division: 5,
        terrain_speed,
        ..Default::default()
    };
    let wide = row(vec![entry(1), entry(256), entry(1581)]);
    let mut bytes = Vec::new();
    wide.encode_fields(&mut bytes).unwrap();
    assert_eq!(
        DivisionEtaInputsRow::decode_fields(wide.key(), &mut Decoder::new(&bytes)).unwrap(),
        wide
    );
    for bad in [
        row(vec![entry(0)]),
        row(vec![entry(-1)]),
        row(vec![entry(256), entry(3), entry(256)]),
    ] {
        assert!(encode_row(&bad, 0, &mut Vec::new()).is_err(), "{bad:?}");
    }
}

#[test]
fn supply_vertex_rows_round_trip_beside_nodes_and_delta_only_their_edge() {
    let node = SupplyNodeRow {
        country: 7,
        index: 3,
        province: 12,
        kind: 2,
        available: 500000,
        parent: 40,
        route_from: 40,
        route_to: 12,
        path: vec![12, 11, 40],
        ..Default::default()
    };
    let vertex = SupplyNodeRow {
        country: 7,
        index: SupplyNodeRow::VERTEX | 40,
        province: 40,
        parent: 9,
        ..Default::default()
    };
    assert!(vertex.is_vertex() && !node.is_vertex());
    assert!(node.key() < vertex.key());
    for row in [&node, &vertex] {
        let mut bytes = Vec::new();
        encode_row(row, 0, &mut bytes).unwrap();
        let mut input = Decoder::new(&bytes);
        assert_eq!(&decode_row::<SupplyNodeRow>(0, &mut input).unwrap(), row);
        input.finish().unwrap();
    }
    let moved = SupplyNodeRow {
        parent: 41,
        route_from: 41,
        route_to: 40,
        break_after: 1,
        path: vec![40, 41],
        ..vertex.clone()
    };
    let delta = moved.delta(Some(&vertex)).unwrap().unwrap();
    let mut bytes = Vec::new();
    encode_delta(TableId::SupplyNodes, &delta, 0, &mut bytes).unwrap();
    let mut input = Decoder::new(&bytes);
    let decoded = decode_delta(TableId::SupplyNodes, 0, &mut input).unwrap();
    input.finish().unwrap();
    assert_eq!(decoded.apply(Some(vertex)).unwrap(), moved);
}

fn round_trip<T: RowCodec + Debug + PartialEq>(row: &T) -> usize {
    let mut bytes = Vec::new();
    encode_row(row, 0, &mut bytes).unwrap();
    let mut input = Decoder::new(&bytes);
    assert_eq!(&decode_row::<T>(0, &mut input).unwrap(), row);
    input.finish().unwrap();
    bytes.len()
}

/// The whole native list is recorded, far past the former 16-entry cap.
#[test]
fn naval_base_country_lists_keep_twenty_thousand_entries_in_native_order() {
    let previous = NavalBaseRow {
        province: 158,
        country: 102,
        level: 10,
        healthy_level: 9,
        countries: (0..20_000_u32).rev().map(|index| index * 3 + 1).collect(),
        countries_unreadable: false,
    };
    round_trip(&previous);
    let mut next = previous.clone();
    next.countries[19_999] = u32::MAX;
    next.countries.push(7);
    let delta = next.delta(Some(&previous)).unwrap().unwrap();
    assert_eq!(delta.mask, 1 << 3);
    let mut bytes = Vec::new();
    encode_delta(TableId::NavalBases, &delta, 0, &mut bytes).unwrap();
    let decoded = decode_delta(TableId::NavalBases, 0, &mut Decoder::new(&bytes)).unwrap();
    assert_eq!(decoded.apply(Some(previous)).unwrap(), next);
}

/// Zero bytes are omitted, in-array gaps read as zero and out-of-array
/// indices as absent; one native array has exactly one encoding.
#[test]
fn sparse_bytes_keep_native_length_nonzero_runs_and_refuse_noncanonical_forms() {
    use crate::value::Value;
    let mut dense = vec![0_u8; 5000];
    for (index, byte) in dense.iter_mut().enumerate() {
        if index % 97 == 0 || (1000..1004).contains(&index) {
            *byte = (index % 251 + 1) as u8;
        }
    }
    dense[4999] = 255;
    let sparse = SparseBytes::from_dense(&dense).unwrap();
    assert_eq!(sparse.len, 5000);
    let mut restored = vec![0; sparse.len as usize];
    for run in &sparse.runs {
        let start = run.start as usize;
        restored[start..start + run.bytes.len()].copy_from_slice(&run.bytes);
    }
    assert_eq!(restored, dense);
    for index in [0, 1, 96, 97, 999, 1000, 1003, 1004, 4998, 4999] {
        assert_eq!(sparse.get(index), Some(dense[index as usize]), "{index}");
    }
    assert_eq!(sparse.get(5000), None);
    assert_eq!(SparseBytes::default().get(0), None);
    let mut bytes = Vec::new();
    sparse.encode(&mut bytes).unwrap();
    assert!(bytes.len() < dense.len() / 10, "{} bytes", bytes.len());
    let mut input = Decoder::new(&bytes);
    assert_eq!(SparseBytes::decode(&mut input).unwrap(), sparse);
    input.finish().unwrap();
    // An all-nonzero array costs only its header and one run.
    let full = SparseBytes::from_dense(&[9; 300]).unwrap();
    let mut bytes = Vec::new();
    full.encode(&mut bytes).unwrap();
    assert_eq!(bytes.len(), 8 + 1 + 2 + 300);

    let run = |start, bytes: &[u8]| ByteRun {
        start,
        bytes: bytes.to_vec(),
    };
    for runs in [
        vec![run(0, &[])],
        vec![run(0, &[1, 0])],
        vec![run(0, &[1]), run(1, &[2])],
        vec![run(2, &[1]), run(0, &[2])],
        vec![run(9, &[1, 2])],
    ] {
        let value = SparseBytes { len: 10, runs };
        assert!(value.encode(&mut Vec::new()).is_err(), "{value:?}");
    }
    for wire in [
        // A second run with no gap would touch the first.
        [
            &10_u32.to_le_bytes()[..],
            &2_u32.to_le_bytes(),
            &[0, 1, 5, 0, 1, 6],
        ]
        .concat(),
        // A zero byte inside a run.
        [
            &10_u32.to_le_bytes()[..],
            &1_u32.to_le_bytes(),
            &[0, 2, 5, 0],
        ]
        .concat(),
        // A run ending past the array.
        [
            &2_u32.to_le_bytes()[..],
            &1_u32.to_le_bytes(),
            &[1, 2, 5, 6],
        ]
        .concat(),
        // An overlong gap.
        [
            &10_u32.to_le_bytes()[..],
            &1_u32.to_le_bytes(),
            &[0x80, 0, 1, 5],
        ]
        .concat(),
    ] {
        assert!(SparseBytes::decode(&mut Decoder::new(&wire)).is_err());
    }
}

/// Five thousand owners over five thousand slots: one nonzero record each
/// plus the zero last record, and one owner with a ten-record array. The
/// counts come back from the rows alone; an in-range index without a row
/// is a zero record and an index past the count is no record at all.
#[test]
fn sparse_country_intel_for_five_thousand_owners_keeps_counts_and_zero_gaps() {
    const SLOTS: u32 = 5000;
    let mut rows = Vec::new();
    for owner in 1..SLOTS {
        let count = if owner == 7 { 10 } else { SLOTS };
        let hit = owner % count;
        rows.push(CountryIntelRow {
            owner_country: owner,
            target_index: hit,
            army: -(owner as i64),
            ..Default::default()
        });
        if hit != count - 1 {
            rows.push(CountryIntelRow {
                owner_country: owner,
                target_index: count - 1,
                ..Default::default()
            });
        }
    }
    rows.sort_by_key(CountryIntelRow::key);
    let mut bytes = Vec::new();
    let mut previous_key = 0;
    for row in &rows {
        encode_row(row, previous_key, &mut bytes).unwrap();
        previous_key = row.key();
    }
    let mut input = Decoder::new(&bytes);
    let mut previous_key = 0;
    let decoded: Vec<CountryIntelRow> = rows
        .iter()
        .map(|_| {
            let row = decode_row::<CountryIntelRow>(previous_key, &mut input).unwrap();
            previous_key = row.key();
            row
        })
        .collect();
    input.finish().unwrap();
    assert_eq!(decoded, rows);
    // Dense rows would be 25 million (about 900 MB); this is two rows per owner.
    assert!(bytes.len() < 80 * SLOTS as usize, "{} bytes", bytes.len());

    let counts = CountryIntelRow::native_counts(&decoded);
    assert_eq!(counts.len(), SLOTS as usize - 1);
    assert_eq!(counts[&1], SLOTS);
    assert_eq!(counts[&7], 10);
    assert_eq!(counts.get(&SLOTS), None);
    let record = |owner: u32, index: u32| {
        decoded
            .iter()
            .find(|row| row.owner_country == owner && row.target_index == index)
    };
    // In range with no row: four exact zeros.
    assert!(record(1, 2).is_none() && 2 < counts[&1]);
    assert_eq!(record(1, 1).unwrap().army, -1);
    // Past the count: not in the native array, which the count alone decides.
    assert!(record(7, 20).is_none() && 20 >= counts[&7]);
}

#[test]
fn train_inventory_access_for_five_thousand_controllers_round_trips_sparsely() {
    let mut dense = vec![0_u8; 5000];
    for slot in [1, 2, 3, 700, 4999] {
        dense[slot] = 1;
    }
    let previous = TrainInventoryRow {
        country: 42,
        node_count: 3,
        allocations: vec![TrainAllocation {
            variant: u64::MAX,
            count: i32::MIN,
        }],
        controller_access: SparseBytes::from_dense(&dense).unwrap(),
    };
    assert!(round_trip(&previous) < 64);
    dense[701] = 128;
    let next = TrainInventoryRow {
        controller_access: SparseBytes::from_dense(&dense).unwrap(),
        ..previous.clone()
    };
    assert_eq!(next.controller_access.runs.len(), 3);
    let delta = next.delta(Some(&previous)).unwrap().unwrap();
    assert_eq!(delta.mask, 1 << 2);
    assert_eq!(delta.apply(Some(previous)).unwrap(), next);
}

#[test]
fn country_appearance_preserves_float_bits_and_literal_tag_transitions() {
    let previous = CountryAppearanceRow {
        country: u32::MAX,
        red: 0x8000_0000,   // Negative zero.
        green: 0x7fc1_2345, // NaN payload.
        blue: 0x7f80_0000,  // Infinity.
        alpha: 0xff80_0000, // Negative infinity.
        cosmetic_tag: b"BRM_british".to_vec(),
        name_tag: b"GER".to_vec(),
        leader_ideology: b"nazism".to_vec(),
        overlord: -1,
        graphical_culture: b"western_european_gfx".to_vec(),
        leader_character: Some(u32::MAX),
        leader_traits: vec![0, u32::MAX, 7],
        old_name: Some(b"Deutsches Reich".to_vec()),
        old_adjective: Some(Vec::new()),
        old_definite_name: None,
        ui_red: 0x3f80_0000,
        ui_green: 0x0000_0001, // Denormal.
        ui_blue: 0x7fc0_0001,  // NaN payload.
        ui_alpha: 0xbf80_0000, // Negative one.
    };
    let mut bytes = Vec::new();
    encode_row(&previous, 0, &mut bytes).unwrap();
    assert_eq!(
        decode_row::<CountryAppearanceRow>(0, &mut Decoder::new(&bytes)).unwrap(),
        previous
    );
    let next = CountryAppearanceRow {
        cosmetic_tag: Vec::new(),
        ..previous.clone()
    };
    let delta = next.delta(Some(&previous)).unwrap().unwrap();
    assert_eq!(delta.mask, 1 << 4);
    assert_eq!(delta.apply(Some(previous.clone())).unwrap(), next);
    // A leader change and a missing leader character stay distinct from zero.
    for leader_character in [Some(0), None] {
        let next = CountryAppearanceRow {
            leader_character,
            ..previous.clone()
        };
        let mut bytes = Vec::new();
        encode_row(&next, 0, &mut bytes).unwrap();
        assert_eq!(
            decode_row::<CountryAppearanceRow>(0, &mut Decoder::new(&bytes)).unwrap(),
            next
        );
        let delta = next.delta(Some(&previous)).unwrap().unwrap();
        assert_eq!(delta.mask, 1 << 9);
        assert_eq!(delta.apply(Some(previous.clone())).unwrap(), next);
    }
    // An empty old name, a missing one and a rename stay distinct.
    for old_name in [Some(Vec::new()), None, Some(b"Reich".to_vec())] {
        let next = CountryAppearanceRow {
            old_name,
            ..previous.clone()
        };
        let mut bytes = Vec::new();
        encode_row(&next, 0, &mut bytes).unwrap();
        assert_eq!(
            decode_row::<CountryAppearanceRow>(0, &mut Decoder::new(&bytes)).unwrap(),
            next
        );
        let delta = next.delta(Some(&previous)).unwrap().unwrap();
        assert_eq!(delta.mask, 1 << 11);
        assert_eq!(delta.apply(Some(previous.clone())).unwrap(), next);
    }
    // A cosmetic change rewrites the UI colour: its own column, not the primary's.
    let recoloured = CountryAppearanceRow {
        ui_green: 0x3f00_0000,
        ..previous.clone()
    };
    let delta = recoloured.delta(Some(&previous)).unwrap().unwrap();
    assert_eq!(delta.mask, 1 << 15);
    assert_eq!(delta.apply(Some(previous.clone())).unwrap(), recoloured);
}

#[test]
fn ship_models_preserve_full_flags_and_counter_inputs() {
    let row = ShipModelRow {
        formation_flags: u64::MAX,
        counter: Some(ShipCounterInputs {
            roster_ordinal: 7,
            definition_valid: 255,
            carrier_air_base: u64::MAX,
            maximum_strength: i64::MIN,
            anti_air_base: i64::MIN + 1,
            maximum_organisation: i64::MAX,
            positive_equipment_flags: u64::MAX,
        }),
        ..Default::default()
    };
    let mut bytes = Vec::new();
    encode_row(&row, 0, &mut bytes).unwrap();
    assert_eq!(
        decode_row::<ShipModelRow>(0, &mut Decoder::new(&bytes)).unwrap(),
        row
    );
}

#[test]
fn combat_window_operands_round_trip() {
    let sides = LandCombatSideDetails {
        commander_formation: u64::MAX,
        commander: 0x8000_0041_0000_0035,
        ..LandCombatSideDetails::default()
    };
    let details = LandCombatDetailsRow {
        combat: 9,
        sides: [sides, LandCombatSideDetails::default()],
        ..LandCombatDetailsRow::default()
    };
    let mut bytes = Vec::new();
    encode_row(&details, 0, &mut bytes).unwrap();
    assert_eq!(
        decode_row::<LandCombatDetailsRow>(0, &mut Decoder::new(&bytes)).unwrap(),
        details
    );
    let division = LandCombatDivisionRow {
        combat: 9,
        side: 1,
        ordinal: u32::MAX,
        direction_group: i32::MIN,
        categories: u64::MAX,
        exile: Some(LandCombatExileInputs {
            origin_type: 2,
            country: -1,
            country_fallback: 7,
            location_cores: vec![7, 0],
            occupiers: vec![u32::MAX],
        }),
        ..LandCombatDivisionRow::default()
    };
    bytes.clear();
    encode_row(&division, 0, &mut bytes).unwrap();
    assert_eq!(
        decode_row::<LandCombatDivisionRow>(0, &mut Decoder::new(&bytes)).unwrap(),
        division
    );
    assert_eq!(TableId::LandCombatDivisions.full_mask(), (1 << 30) - 1);
    assert_eq!(TableId::LandCombatDivisions.mask_width(), 4);
}

#[test]
fn persisted_upgrade_groups_keep_army_identity_and_native_priority() {
    let mut previous = ArmyUpgradeGroupRow {
        country: u32::MAX,
        army: u64::MAX,
        priority: 1,
    };
    assert_eq!(TableId::ArmyUpgradeGroups.full_mask(), 3);
    for priority in [0, u32::MAX, 1] {
        let next = ArmyUpgradeGroupRow {
            priority,
            ..previous.clone()
        };
        let delta = next.delta(Some(&previous)).unwrap().unwrap();
        assert_eq!(delta.mask, 2);
        assert_eq!(delta.key, u128::from(u64::MAX));
        assert_eq!(delta.apply(Some(previous)).unwrap(), next);
        let mut bytes = Vec::new();
        encode_row(&next, 0, &mut bytes).unwrap();
        let mut input = Decoder::new(&bytes);
        assert_eq!(input.varint().unwrap(), u128::from(u64::MAX));
        assert_eq!(input.remaining(), 8);
        assert_eq!(input.u32().unwrap(), u32::MAX);
        assert_eq!(input.u32().unwrap(), priority);
        input.finish().unwrap();
        assert_eq!(
            decode_row::<ArmyUpgradeGroupRow>(0, &mut Decoder::new(&bytes)).unwrap(),
            next
        );
        previous = next;
    }
}

#[test]
fn operations_record_native_definition_and_independent_progress_fields() {
    let previous = OperationRow {
        id: 7,
        owner: 1,
        target: 2,
        definition_token: 0x1234_5678,
        preparation_current: i64::MIN,
        preparation_required: i64::MAX,
        ..OperationRow::default()
    };
    assert_eq!(TableId::Operations.field_count(), 13);
    let mut bytes = Vec::new();
    encode_row(&previous, 0, &mut bytes).unwrap();
    assert_eq!(bytes.len(), 49);
    assert_eq!(
        &bytes[..14],
        &[7, 1, 0, 0, 0, 2, 0, 0, 0, 0, 0x78, 0x56, 0x34, 0x12]
    );
    assert_eq!(&bytes[14..22], &i64::MIN.to_le_bytes());
    assert_eq!(&bytes[22..30], &i64::MAX.to_le_bytes());
    let next = OperationRow {
        preparation_current: i64::MAX,
        assigned_slots: u32::MAX,
        ..previous.clone()
    };
    let delta = next.delta(Some(&previous)).unwrap().unwrap();
    assert_eq!(delta.mask, (1 << 4) | (1 << 12));
    assert_eq!(delta.apply(Some(previous)).unwrap(), next);
}

#[test]
fn intel_networks_preserve_unassigned_selectors_and_optional_native_strength() {
    let mut previous = IntelNetworkRow {
        owner_country: u32::MAX,
        target_country: 7,
        state_id: i32::MIN,
        strength_raw: None,
        subnetwork_selector: -1,
        local_strength_raw: i64::MIN,
    };
    for (subnetwork_selector, strength_raw, mask) in [
        (i32::MIN, None, 2),
        (-2, None, 2),
        (0, Some(0), 3),
        (0, Some(i64::MIN), 1),
        (0, Some(i64::MAX), 1),
        (i32::MAX, Some(i64::MAX), 2),
        (-1, None, 3),
    ] {
        let next = IntelNetworkRow {
            strength_raw,
            subnetwork_selector,
            ..previous.clone()
        };
        let delta = next.delta(Some(&previous)).unwrap().unwrap();
        assert_eq!(delta.mask, mask);
        assert_eq!(delta.apply(Some(previous)).unwrap(), next);
        let mut bytes = Vec::new();
        encode_row(&next, 0, &mut bytes).unwrap();
        let mut input = Decoder::new(&bytes);
        assert_eq!(decode_row::<IntelNetworkRow>(0, &mut input).unwrap(), next);
        input.finish().unwrap();
        previous = next;
    }
}

#[test]
fn theatre_volunteer_tag_preserves_signed_bits_independently_of_owner() {
    let mut previous = LandTheatreRow {
        id: u64::MAX,
        owner: u32::MAX,
        ordinal: 7,
        volunteers_theatre: i32::MIN,
    };
    assert_eq!(TableId::LandTheatres.full_mask(), 7);
    for volunteers_theatre in [i32::MAX, 0, -1, i32::MIN] {
        let next = LandTheatreRow {
            volunteers_theatre,
            ..previous.clone()
        };
        let delta = next.delta(Some(&previous)).unwrap().unwrap();
        assert_eq!(delta.mask, 1 << 2);
        assert_eq!(delta.apply(Some(previous)).unwrap(), next);
        let mut wire = Vec::new();
        encode_row(&next, 0, &mut wire).unwrap();
        let mut input = Decoder::new(&wire);
        assert_eq!(input.varint().unwrap(), u128::from(u64::MAX));
        assert_eq!(input.u32().unwrap(), u32::MAX);
        assert_eq!(input.u32().unwrap(), 7);
        assert_eq!(input.i32().unwrap(), volunteers_theatre);
        input.finish().unwrap();
        previous = next;
    }
}

#[test]
fn wide_masks_change_one_native_channel_at_a_time() {
    assert_eq!(TableId::AirStatistics.field_count(), 26);
    assert_eq!(TableId::AirStatistics.mask_width(), 4);
    let previous = AirStatisticsRow::default();
    for index in 0..24 {
        let mut next = previous.clone();
        next.channels[index] = i64::MIN;
        let delta = next.delta(Some(&previous)).unwrap().unwrap();
        assert_eq!(delta.mask, 1_u64 << (index + 2));
        let mut bytes = Vec::new();
        encode_delta(TableId::AirStatistics, &delta, 0, &mut bytes).unwrap();
        assert_eq!(&bytes[1..5], &(1_u32 << (index + 2)).to_le_bytes());
        assert_eq!(delta.apply(Some(previous.clone())).unwrap(), next);
    }
    let previous = VariantStatsRow::default();
    for index in 0..10 {
        let mut next = previous.clone();
        next.values[index] = i64::MAX;
        let delta = next.delta(Some(&previous)).unwrap().unwrap();
        assert_eq!(delta.mask, 1_u64 << index);
        assert_eq!(delta.apply(Some(previous.clone())).unwrap(), next);
    }
    let previous = ProjectRow::default();
    let mut next = previous.clone();
    next.state.state_enums[3] = u32::MAX;
    let delta = next.delta(Some(&previous)).unwrap().unwrap();
    assert_eq!(delta.mask.count_ones(), 1);
    assert_eq!(delta.apply(Some(previous)).unwrap(), next);
}

#[test]
fn structured_fields_reject_hostile_inner_counts_tags_and_suffixes() {
    fn reject<T: NativeValue>(inner: &[u8]) {
        let mut bytes = Vec::new();
        put_bytes(&mut bytes, inner).unwrap();
        assert!(read_value::<T>(Kind::Opaque, &mut Decoder::new(&bytes)).is_err());
    }
    reject::<Option<Vec<u8>>>(&[2]);
    reject::<Option<Vec<u8>>>(&[1, 255, 255, 255, 255]);
    reject::<Option<Vec<u8>>>(&[0, 0]);
    reject::<Option<AceDeath>>(&[1, 0]);
    reject::<Option<AceDeath>>(&[2]);
    reject::<Vec<i64>>(&u32::MAX.to_le_bytes());
    reject::<Vec<ScriptVariableEntry>>(&u32::MAX.to_le_bytes());
    reject::<Vec<AirHistoryEntry>>(&u32::MAX.to_le_bytes());
    reject::<Vec<AirDamageEntry>>(&u32::MAX.to_le_bytes());
    reject::<Vec<SunkShipEntry>>(&u32::MAX.to_le_bytes());
    let mut nested = 1_u32.to_le_bytes().to_vec();
    nested.extend_from_slice(&u32::MAX.to_le_bytes());
    nested.extend_from_slice(&0_i64.to_le_bytes());
    reject::<Vec<ScriptVariableEntry>>(&nested);
    for kind in [Kind::OptionalI32, Kind::OptionalU64, Kind::Bool] {
        assert!(kind.decode(&mut Decoder::new(&[2])).is_err());
    }
    for (kind, old, difference) in [
        (Kind::I16, Field::I16(i16::MAX), 1),
        (Kind::I16, Field::I16(i16::MIN), -1),
        (Kind::U16, Field::U16(u16::MAX), 1),
        (Kind::U16, Field::U16(0), -1),
        (Kind::Bool, Field::Bool(true), 1),
        (Kind::OptionalI32, Field::OptionalI32(Some(i32::MIN)), -1),
        (Kind::OptionalU64, Field::OptionalU64(Some(u64::MAX)), 1),
    ] {
        assert!(Change::Number(difference).apply(kind, old).is_err());
    }
}

#[test]
fn nested_allocations_move_unchanged_and_absence_differs_from_empty() {
    let previous = AirAceRow {
        name: vec![0, 255, b'N'],
        surname: vec![254],
        callsign: vec![0],
        death: Some(AceDeath {
            killer_ace: u64::MAX,
            killer_country: u32::MAX,
        }),
        ..AirAceRow::default()
    };
    let name = previous.name.as_ptr();
    let mut next = previous.clone();
    next.handled = 255;
    let delta = next.delta(Some(&previous)).unwrap().unwrap();
    let restored = delta.apply(Some(previous)).unwrap();
    assert_eq!(restored, next);
    assert_eq!(restored.name.as_ptr(), name);
    let mut previous = FocusRow::default();
    for tree in [Some(vec![]), None, Some(vec![0, 255]), Some(vec![])] {
        let next = FocusRow {
            tree,
            ..FocusRow::default()
        };
        let delta = next.delta(Some(&previous)).unwrap().unwrap();
        assert_eq!(delta.mask, 1);
        assert_eq!(delta.apply(Some(previous)).unwrap(), next);
        previous = next;
    }
}

#[test]
fn state_flag_payload_is_one_lossless_byte_and_unchanged_emits_no_delta() {
    let mut previous = StateFlagRow {
        state: 978,
        demilitarized: 0,
    };
    for byte in [0, 1, 255, 2, 0] {
        let row = StateFlagRow {
            state: 978,
            demilitarized: byte,
        };
        let mut payload = Vec::new();
        row.encode_fields(&mut payload).unwrap();
        assert_eq!(payload, [byte]);
        assert_eq!(
            StateFlagRow::decode_fields(978, &mut Decoder::new(&payload)).unwrap(),
            row
        );
        if row == previous {
            assert!(row.delta(Some(&previous)).unwrap().is_none());
        } else {
            let delta = row.delta(Some(&previous)).unwrap().unwrap();
            assert_eq!(delta.mask, 1);
            assert_eq!(delta.apply(Some(previous)).unwrap(), row);
        }
        previous = row;
    }
}

#[test]
fn every_extended_table_round_trips_extremes_nested_records_and_deltas() {
    check_table::<FocusRow>(TableId::Focus);
    check_table::<CompletedFocusRow>(TableId::CompletedFocuses);
    check_table::<ResearchTechnologyRow>(TableId::ResearchTechnologies);
    check_table::<ResearchSlotRow>(TableId::ResearchSlots);
    check_table::<TechnologyDefinitionRow>(TableId::TechnologyDefinitions);
    check_table::<PoliticsRow>(TableId::Politics);
    check_table::<PoliticalPartyRow>(TableId::PoliticalParties);
    check_table::<ActiveIdeaRow>(TableId::ActiveIdeas);
    check_table::<DecisionCountsRow>(TableId::DecisionCounts);
    check_table::<DecisionRow>(TableId::Decisions);
    check_table::<DynamicModifierRow>(TableId::DynamicModifiers);
    check_table::<PreferredTacticRow>(TableId::PreferredTactics);
    check_table::<CountryTacticUnlockRow>(TableId::CountryTacticUnlocks);
    check_table::<TechSharingGroupRow>(TableId::TechSharingGroups);
    check_table::<TechSharingMemberRow>(TableId::TechSharingMembers);
    check_table::<PowerBalanceRow>(TableId::PowerBalances);
    check_table::<PowerBalanceMemberRow>(TableId::PowerBalanceMembers);
    check_table::<PowerBalanceRangeRow>(TableId::PowerBalanceRanges);
    check_table::<PowerBalanceOverrideRow>(TableId::PowerBalanceOverrides);
    check_table::<ProductionLineRow>(TableId::ProductionLines);
    check_table::<StockpileRow>(TableId::Stockpiles);
    check_table::<DivisionEquipmentRow>(TableId::DivisionEquipment);
    check_table::<VariantDefinitionRow>(TableId::VariantDefinitions);
    check_table::<VariantModuleRow>(TableId::VariantModules);
    check_table::<VariantStatsRow>(TableId::VariantStats);
    check_table::<VariantUpgradeRow>(TableId::VariantUpgrades);
    check_table::<ArmyUpgradeGroupRow>(TableId::ArmyUpgradeGroups);
    check_table::<ArmyUpgradeRequestRow>(TableId::ArmyUpgradeRequests);
    check_table::<ArmyUpgradeDeliveryRow>(TableId::ArmyUpgradeDeliveries);
    check_table::<ArmyUpgradeProducedRow>(TableId::ArmyUpgradeProduced);
    check_table::<CountryFuelRow>(TableId::CountryFuel);
    check_table::<CountryFuelConsumerRow>(TableId::CountryFuelConsumers);
    check_table::<ResourceDefinitionRow>(TableId::ResourceDefinitions);
    check_table::<CountryResourceRow>(TableId::CountryResources);
    check_table::<ResourceExchangeRow>(TableId::ResourceExchanges);
    check_table::<PurchaseContractRow>(TableId::PurchaseContracts);
    check_table::<ProvinceSupplyRow>(TableId::ProvinceSupply);
    check_table::<HubMotorizationRow>(TableId::HubMotorization);
    check_table::<DisruptedSupplyRow>(TableId::DisruptedSupply);
    check_table::<ForeignHomebaseRow>(TableId::ForeignHomebases);
    check_table::<CountryExperienceRow>(TableId::CountryExperience);
    check_table::<BuildingRow>(TableId::StateBuildings);
    check_table::<BuildingRow>(TableId::ProvinceBuildings);
    check_table::<StateBaseResourceRow>(TableId::StateBaseResources);
    check_table::<BuildingConstructionRow>(TableId::BuildingConstruction);
    check_table::<RailwayConstructionRow>(TableId::RailwayConstruction);
    check_table::<OccupationSettingsRow>(TableId::OccupationSettings);
    check_table::<StateOccupationLawRow>(TableId::StateOccupationLaws);
    check_table::<StateGarrisonRow>(TableId::StateGarrisons);
    check_table::<StateAssignedManpowerRow>(TableId::StateAssignedManpower);
    check_table::<CountryOccupationReserveManpowerRow>(TableId::CountryOccupationReserveManpower);
    check_table::<StateResistanceRow>(TableId::StateResistance);
    check_table::<CountryCollaborationRow>(TableId::CountryCollaboration);
    check_table::<CharacterRow>(TableId::Characters);
    check_table::<CountryLeaderRoleRow>(TableId::CountryLeaderRoles);
    check_table::<AdvisorRoleRow>(TableId::AdvisorRoles);
    check_table::<UnitLeaderRow>(TableId::UnitLeaders);
    check_table::<UnitLeaderTraitRow>(TableId::UnitLeaderTraits);
    check_table::<UnitLeaderTraitExperienceRow>(TableId::UnitLeaderTraitExperience);
    check_table::<ActiveAbilityRow>(TableId::ActiveAbilities);
    check_table::<CapitulationRow>(TableId::Capitulation);
    check_table::<SubjectRow>(TableId::Subjects);
    check_table::<AutonomyRow>(TableId::Autonomy);
    check_table::<IntelligenceAgencyRow>(TableId::IntelligenceAgencies);
    check_table::<OperativeMembershipRow>(TableId::OperativeMemberships);
    check_table::<CryptologyRow>(TableId::Cryptology);
    check_table::<IntelNetworkRow>(TableId::IntelNetworks);
    check_table::<ScriptFlagRow>(TableId::ScriptFlags);
    check_table::<ScriptVariablesRow>(TableId::ScriptVariables);
    check_table::<GlobalSavedEventTargetRow>(TableId::GlobalSavedEventTargets);
    check_table::<OperationRow>(TableId::Operations);
    check_table::<OperationAssignmentRow>(TableId::OperationAssignments);
    check_table::<WingEquipmentRow>(TableId::WingEquipment);
    check_table::<AirBaseRow>(TableId::AirBases);
    check_table::<WingCumulativeRow>(TableId::WingCumulative);
    check_table::<AirRegionHistoryRow>(TableId::AirRegionHistory);
    check_table::<AirRegionDamageRow>(TableId::AirRegionDamage);
    check_table::<AirAceRow>(TableId::AirAces);
    check_table::<AirStatisticsRow>(TableId::AirStatistics);
    check_table::<SunkConvoyRow>(TableId::SunkConvoys);
    check_table::<RailCooldownRow>(TableId::RailCooldowns);
    check_table::<RailEdgeRow>(TableId::RailEdges);
    check_table::<LandTheatreRow>(TableId::LandTheatres);
    check_table::<LandTheatreGroupRow>(TableId::LandTheatreGroups);
    check_table::<LandTheatreMemberRow>(TableId::LandTheatreMembers);
    check_table::<AirTheatreRow>(TableId::AirTheatres);
    check_table::<AirTheatreGroupRow>(TableId::AirTheatreGroups);
    check_table::<AirTheatreMemberRow>(TableId::AirTheatreMembers);
    check_table::<NavalResultRow>(TableId::NavalResults);
    check_table::<NavalResultMemberRow>(TableId::NavalResultMembers);
    check_table::<NavalResultCountryRow>(TableId::NavalResultCountries);
    check_table::<SunkShipsRow>(TableId::SunkShips);
    check_table::<RaidTooltipOperandRow>(TableId::RaidTooltipOperands);
    check_table::<CountryCharacterRow>(TableId::CountryRetiredCharacters);
    check_table::<CharacterRoleTraitRow>(TableId::CharacterRoleTraits);
    check_table::<CharacterOperativeRow>(TableId::CharacterOperatives);
    check_table::<RaidIntelInputRow>(TableId::RaidIntelInputs);
    check_table::<RaidAirScoreRow>(TableId::RaidAirScores);
    check_table::<RaidFacilityRow>(TableId::RaidFacilities);
    check_table::<ProjectProgramRow>(TableId::ProjectPrograms);
    check_table::<ProjectRow>(TableId::Projects);
    check_table::<ProjectHistoryRow>(TableId::ProjectHistory);
    check_table::<ProjectBreakthroughRow>(TableId::ProjectBreakthroughs);
    check_table::<NukeRow>(TableId::Nukes);
    check_table::<RaidCountryRow>(TableId::RaidCountries);
    check_table::<RaidRow>(TableId::Raids);
    check_table::<RaidTargetRow>(TableId::RaidTargets);
    check_table::<ResearchModifierRow>(TableId::ResearchModifiers);
    check_table::<ResearchInputRow>(TableId::ResearchInputs);
    check_table::<ResearchAppliedBonusRow>(TableId::ResearchAppliedBonuses);
    check_table::<NavalBaseRow>(TableId::NavalBases);
    check_table::<PeaceConferenceRow>(TableId::PeaceConferences);
    check_table::<WeatherRegionRow>(TableId::WeatherRegions);
    check_table::<WeatherProvinceRow>(TableId::WeatherProvinces);
    check_table::<OperationTokensRow>(TableId::OperationTokens);
    check_table::<LexicalTokenRow>(TableId::LexicalTokens);
    check_table::<NavalSupremacyRow>(TableId::NavalSupremacy);
    check_table::<RadarCoverageRow>(TableId::RadarCoverage);
    check_table::<SentVolunteerRow>(TableId::SentVolunteers);
    check_table::<VolunteerHostRow>(TableId::VolunteerHosts);
    check_table::<TheatreAreaRow>(TableId::TheatreAreas);
    check_table::<StationedUnitRow>(TableId::StationedUnits);
    check_table::<UnitIntelRow>(TableId::UnitIntel);
    check_table::<AirBaseRegistrationRow>(TableId::AirBaseRegistrations);
    check_table::<ProductionLicenseRow>(TableId::ProductionLicenses);
    check_table::<ProductionLicenseRecordRow>(TableId::ProductionLicenseRecords);
    check_table::<DeploymentConveyorRow>(TableId::DeploymentConveyors);
    check_table::<DeploymentLineRow>(TableId::DeploymentLines);
    check_table::<DeploymentManpowerRow>(TableId::DeploymentManpower);
    check_table::<DeploymentEquipmentRow>(TableId::DeploymentEquipment);
    check_table::<WorldTensionRow>(TableId::WorldTension);
    check_table::<ProvinceNameRow>(TableId::ProvinceNames);
    check_table::<StateFlagRow>(TableId::StateFlags);
    check_table::<ArmyOrderPlanRow>(TableId::ArmyOrderPlans);
    check_table::<OrdersGroupPlanRow>(TableId::OrdersGroupPlans);
    check_table::<FrontRow>(TableId::Fronts);
    check_table::<CountryAppearanceRow>(TableId::CountryAppearance);
    check_table::<ArmyIconInputRow>(TableId::ArmyIconInputs);
    check_table::<ShipModelRow>(TableId::ShipModels);
    check_table::<ProvinceUnitOrderRow>(TableId::ProvinceUnitOrder);
    check_table::<CharacterPortraitRow>(TableId::CharacterPortraits);
    check_table::<LeaderPortraitRow>(TableId::LeaderPortraits);
    check_table::<TaskForceCounterInputRow>(TableId::TaskForceCounterInputs);
    check_table::<RailwayGunCounterInputRow>(TableId::RailwayGunCounterInputs);
    check_table::<TaskForceMapMotionRow>(TableId::TaskForceMapMotion);
    check_table::<FloatingHarborDeploymentRow>(TableId::FloatingHarborDeployments);
    check_table::<FacilityMapFlagRow>(TableId::FacilityMapFlags);
    check_table::<NavalProvinceCombatRow>(TableId::NavalProvinceCombats);
    check_table::<NavalCombatTransferRow>(TableId::NavalCombatTransfers);
    check_table::<NavalTransferUnitRow>(TableId::NavalTransferUnits);
    check_table::<TaskForceMissionRow>(TableId::TaskForceMissions);
    check_table::<NavalCombatAirBaseWingHandlesRow>(TableId::NavalCombatAirBaseWingHandles);
    check_table::<FleetCounterInputRow>(TableId::FleetCounterInputs);
    check_table::<CountryCounterInputRow>(TableId::CountryCounterInputs);
    check_table::<OrdersGroupCounterInputRow>(TableId::OrdersGroupCounterInputs);
    check_table::<LeaderPortraitPathRow>(TableId::LeaderPortraitPaths);
    check_table::<SupplyNodeRow>(TableId::SupplyNodes);
    check_table::<LandCombatDetailsRow>(TableId::LandCombatDetails);
    check_table::<LandCombatDivisionRow>(TableId::LandCombatDivisions);
    check_table::<LandCombatModifierRow>(TableId::LandCombatModifiers);
    check_table::<NavalCombatDetailsRow>(TableId::NavalCombatDetails);
    check_table::<NavalCombatShipRow>(TableId::NavalCombatShips);
    check_table::<NavalCombatResultDetailsRow>(TableId::NavalCombatResultDetails);
    check_table::<TrainInventoryRow>(TableId::TrainInventory);
    check_table::<LocalPlayerRow>(TableId::LocalPlayer);
    check_table::<SimulationFeatureRow>(TableId::SimulationFeatures);
    check_table::<CountryVisibilityRow>(TableId::CountryVisibility);
    check_table::<StateVisibilityRow>(TableId::StateVisibility);
    check_table::<RaidTargetStatusRow>(TableId::RaidTargetStatuses);
    check_table::<RaidCooldownRow>(TableId::RaidCooldowns);
    check_table::<RaidPathDistanceRow>(TableId::RaidPathDistances);
    check_table::<StateResourceInputRow>(TableId::StateResourceInputs);
    check_table::<StateResourceRow>(TableId::StateResources);
    check_table::<StateDynamicModifierRow>(TableId::StateDynamicModifiers);
    check_table::<CountryBuildingResourceRow>(TableId::CountryBuildingResources);
    check_table::<RaidCountryInputRow>(TableId::RaidCountryInputs);
    check_table::<CountryCommandPowerRow>(TableId::CountryCommandPower);
    check_table::<CountryFactoryCountsRow>(TableId::CountryFactoryCounts);
    check_table::<DivisionRetreatBanRow>(TableId::DivisionRetreatBans);
    check_table::<StateCoreRow>(TableId::StateCores);
    check_table::<RaidEquipmentRow>(TableId::RaidEquipment);
    check_table::<InvasionPreparationRow>(TableId::InvasionPreparations);
    check_table::<CountryCharacterRow>(TableId::CountryCharacters);
    check_table::<PartyLeaderRow>(TableId::PartyLeaders);
    check_table::<CountryCivilWarRow>(TableId::CountryCivilWars);
    check_table::<AdvisorTokenRow>(TableId::AdvisorTokens);
    check_table::<FactionAdvisorRow>(TableId::FactionAdvisors);
    check_table::<RaidSpeedInputRow>(TableId::RaidSpeedInputs);
    check_table::<StockpileArchetypeRow>(TableId::StockpileArchetypes);
    check_table::<RaidAirSiteRow>(TableId::RaidAirSites);
    check_table::<AceRangeInputRow>(TableId::AceRangeInputs);
    check_table::<CountryWarRow>(TableId::CountryWars);
    check_table::<NavalRegionAccessRow>(TableId::NavalRegionAccess);
    check_table::<BorderConflictRow>(TableId::BorderConflicts);
    check_table::<RaidPlayerAllowedTypeRow>(TableId::RaidPlayerAllowedTypes);
    check_table::<RelationLinkRow>(TableId::RelationLinks);
    check_table::<SliceReadRow>(TableId::SliceReads);
    check_table::<FactionTheatreRow>(TableId::FactionTheatres);
    check_table::<CountryLocalSubunitRow>(TableId::CountryLocalSubunits);
    check_table::<AvailableEquipmentVariantRow>(TableId::AvailableEquipmentVariants);
    check_table::<DivisionEquipmentRow>(TableId::NewDivisionEquipment);
    check_table::<AirRegionalMissionRow>(TableId::AirRegionalMissions);
    check_table::<AirCountryRelationRow>(TableId::AirCountryRelations);
    check_table::<AirReferenceWitnessRow>(TableId::AirReferenceWitnesses);
    check_table::<NavalMineLevelRow>(TableId::NavalMineLevels);
    check_table::<AirNavalPatrolCombatRow>(TableId::AirNavalPatrolCombats);
    check_table::<AirNavalPatrolHistoryRow>(TableId::AirNavalPatrolHistories);
    check_table::<AirMissionInputRow>(TableId::AirMissionInputs);
    check_table::<AirSortieBaseCountryRow>(TableId::AirSortieBaseCountries);
    check_table::<AirSortieModifierRow>(TableId::AirSortieModifiers);
    check_table::<AirSortieSpecialBaseRow>(TableId::AirSortieSpecialBases);
    check_table::<AirSortieModifierMetadataRow>(TableId::AirSortieModifierMetadata);
    check_table::<ModifierDefinitionRow>(TableId::ModifierDefinitions);
    check_table::<HiredAdvisorRow>(TableId::HiredAdvisors);
    check_table::<CountryStabilityInputRow>(TableId::CountryStabilityInputs);
    check_table::<CountryStabilityLineRow>(TableId::CountryStabilityLines);
    check_table::<NavalMissionTaskForceRow>(TableId::NavalMissionTaskForces);
    check_table::<NavalMissionLedgerRow>(TableId::NavalMissionLedgers);
    check_table::<NavalMissionCountryRow>(TableId::NavalMissionCountries);
    check_table::<NavalMissionCommanderRow>(TableId::NavalMissionCommanders);
    check_table::<DivisionTemplateRow>(TableId::DivisionTemplates);
    check_table::<DivisionTemplateIconRow>(TableId::DivisionTemplateIcons);
    check_table::<DivisionMovementRow>(TableId::DivisionMovement);
    check_table::<DivisionStatusRow>(TableId::DivisionStatus);
    check_table::<DivisionEtaInputsRow>(TableId::DivisionEtaInputs);
    check_table::<LandCombatProgressRow>(TableId::LandCombatProgress);
    check_table::<LandCombatSideRow>(TableId::LandCombatSides);
    check_table::<ThreatSourceRow>(TableId::ThreatSources);
    check_table::<ThreatCountryAggregateRow>(TableId::ThreatCountryAggregates);
    check_table::<RelationOpinionRow>(TableId::RelationOpinions);
    check_table::<RelationTupleRow>(TableId::RelationTuples);
    check_table::<CountryIntelRow>(TableId::CountryIntel);
}
