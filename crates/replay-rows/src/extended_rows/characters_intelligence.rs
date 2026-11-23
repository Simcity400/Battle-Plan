use super::*;
opaque_value!(Option<Option<u32>>);

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
impl OpaqueValue for AdvisorModifierCache {}
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
impl OpaqueValue for OperativeEffect {}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct OperativeEffectModifier {
    pub key: u32,
    pub value: i64,
    pub sources: Option<[std::sync::Arc<Vec<ModifierBreakdownNode>>; 3]>,
}
value_struct!(OperativeEffectModifier { key: u32, value: i64, sources: Option<[std::sync::Arc<Vec<ModifierBreakdownNode>>; 3]> });
opaque_value!(Option<std::sync::Arc<[Vec<u64>; 5]>>);

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
impl OpaqueValue for TradeInfluence {}

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
impl OpaqueValue for DiplomaticPressure {}

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
impl OpaqueValue for OperationRequirement {}

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
impl OpaqueValue for OperationEquipment {}

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
impl OpaqueValue for OperationEquipmentTotal {}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ScientistTemplateOrigin {
    pub file: Vec<u8>,
    pub line: u32,
}
value_struct!(ScientistTemplateOrigin { file: Vec<u8>, line: u32 });
impl OpaqueValue for ScientistTemplateOrigin {}
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
impl OpaqueValue for AgencyBufferedOffer {}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AgencyBufferedOperation {
    pub operation: u64,
    pub state: Option<u32>,
}
value_struct!(AgencyBufferedOperation { operation: u64, state: Option<u32> });
impl OpaqueValue for AgencyBufferedOperation {}
