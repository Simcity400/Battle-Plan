use super::*;
opaque_value!(Option<Option<u32>>);

/// The cache's ordered fixed5 entries.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CachedModifier {
    pub definition: u32,
    pub value: i64,
}
value_struct!(CachedModifier {
    definition: u32,
    value: i64
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AdvisorModifierCache {
    pub entries: Vec<CachedModifier>,
}
value_struct!(AdvisorModifierCache { entries: Vec<CachedModifier> });
opaque_value!(Option<AdvisorModifierCache>);
opaque_value!(Option<Vec<Vec<u8>>>);

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct OperativeEffect {
    pub owned_states: i32,
    pub network_gain_scale: i32,
    pub trade_opinion: Option<i64>,
    pub modifiers: Vec<OperativeEffectModifier>,
}
value_struct!(OperativeEffect { owned_states: i32, network_gain_scale: i32, trade_opinion: Option<i64>, modifiers: Vec<OperativeEffectModifier> });
opaque_value!(Option<std::sync::Arc<OperativeEffect>>);

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct OperativeEffectModifier {
    pub key: u32,
    pub value: i64,
    pub sources: Option<[std::sync::Arc<Vec<ModifierBreakdownNode>>; 3]>,
}
value_struct!(OperativeEffectModifier { key: u32, value: i64, sources: Option<[std::sync::Arc<Vec<ModifierBreakdownNode>>; 3]> });
opaque_value!(Option<std::sync::Arc<[Vec<u64>; 5]>>);

/// One control-trade influence entry: the influencing country's raw
/// reference and its fixed5 influence, saved under
/// `trade_influence_modifier_of_buyers`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct TradeInfluence {
    pub owner: u32,
    pub value: i64,
}
value_struct!(TradeInfluence {
    owner: u32,
    value: i64
});
opaque_value!(Vec<TradeInfluence>);

/// One diplomatic-pressure entry: the pressuring country's raw reference
/// and the raw Q15 acceptance and tension words, saved as `opinion` and
/// `tension` under `diplomatic_pressure_from_countries`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DiplomaticPressure {
    pub owner: u32,
    pub acceptance: i64,
    pub tension: i64,
}
value_struct!(DiplomaticPressure {
    owner: u32,
    acceptance: i64,
    tension: i64
});
opaque_value!(Vec<DiplomaticPressure>);

/// One operation requirement record. An equipment requirement names its
/// equipment type's archetype by lexer token and is saved as
/// `<archetype>=<amount>`; the civilian-factory requirement has no type
/// (`equipment` and `archetype` zero) and is saved as
/// `civilian_factories={ amount days }` with its amount truncated to whole
/// factories. `days` is saved only for the factory requirement, and
/// `uses_time` (the raw uses-preparation-time byte) never.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct OperationRequirement {
    pub amount: i64,
    pub days: i32,
    pub archetype: u32,
    pub equipment: u8,
    pub uses_time: u8,
}
value_struct!(OperationRequirement {
    amount: i64,
    days: i32,
    archetype: u32,
    equipment: u8,
    uses_time: u8
});
opaque_value!(Vec<OperationRequirement>);

/// One equipment pool variant entry: the full variant reference, the lexer
/// token of its equipment type's archetype and its fixed5 amount, in pool
/// order. The save writes the variant and amount.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct OperationEquipment {
    pub variant: u64,
    pub archetype: u32,
    pub amount: i64,
}
value_struct!(OperationEquipment {
    variant: u64,
    archetype: u32,
    amount: i64
});
opaque_value!(Vec<OperationEquipment>);

/// One equipment pool archetype total: the archetype's lexer token and the
/// stored fixed5 total, in native order.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct OperationEquipmentTotal {
    pub archetype: u32,
    pub amount: i64,
}
value_struct!(OperationEquipmentTotal {
    archetype: u32,
    amount: i64
});
opaque_value!(Vec<OperationEquipmentTotal>);

/// Source of the retained scientist role's nonempty visible conjunction.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ScientistTemplateOrigin {
    pub file: Vec<u8>,
    pub line: u32,
}
value_struct!(ScientistTemplateOrigin { file: Vec<u8>, line: u32 });
opaque_value!(Option<ScientistTemplateOrigin>);
opaque_value!(Option<u8>);

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AgencyBufferedOffer {
    pub definition: u32,
    pub country: u32,
    pub state: Option<u32>,
    pub duration: Option<i32>,
}
value_struct!(AgencyBufferedOffer { definition: u32, country: u32, state: Option<u32>, duration: Option<i32> });
opaque_value!(Vec<AgencyBufferedOffer>);

/// A raw buffered reference and the state getter result at this sample. A stale
/// or null reference is None; a resolved operation with no state is Some(0).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AgencyBufferedOperation {
    pub operation: u64,
    pub state: Option<u32>,
}
value_struct!(AgencyBufferedOperation { operation: u64, state: Option<u32> });
opaque_value!(Vec<AgencyBufferedOperation>);