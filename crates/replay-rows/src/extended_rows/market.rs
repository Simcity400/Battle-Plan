use super::*;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct MarketEquipment {
    pub variant: u64,
    pub amount_raw: i64,
}
value_struct!(MarketEquipment {
    variant: u64,
    amount_raw: i64
});
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct MarketPool {
    pub allow_zero: bool,
    pub items: Vec<MarketEquipment>,
}
value_struct!(MarketPool { allow_zero: bool, items: Vec<MarketEquipment> });
opaque_value!(MarketPool);

macro_rules! shared_market_value {
    ($name:ident, $value:ty) => {
        #[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
        pub struct $name(pub std::sync::Arc<$value>);
        impl Default for $name {
            fn default() -> Self {
                static EMPTY: std::sync::OnceLock<std::sync::Arc<$value>> =
                    std::sync::OnceLock::new();
                Self(
                    EMPTY
                        .get_or_init(|| std::sync::Arc::new(<$value>::default()))
                        .clone(),
                )
            }
        }
        impl From<$value> for $name {
            fn from(value: $value) -> Self {
                Self(std::sync::Arc::new(value))
            }
        }
        impl std::ops::Deref for $name {
            type Target = $value;
            fn deref(&self) -> &$value {
                &self.0
            }
        }
        impl std::ops::DerefMut for $name {
            fn deref_mut(&mut self) -> &mut $value {
                std::sync::Arc::make_mut(&mut self.0)
            }
        }
        impl Value for $name {
            const MINIMUM_ENCODED_BYTES: usize = <$value as Value>::MINIMUM_ENCODED_BYTES;
            fn encoded_bytes(&self) -> io::Result<usize> {
                Value::encoded_bytes(&**self)
            }
            fn try_clone(&self) -> io::Result<Self> {
                Ok(self.clone())
            }
            fn heap_bytes(&self) -> usize {
                Value::heap_bytes(&**self)
            }
            fn encode_reserved(&self, output: &mut Vec<u8>) -> io::Result<()> {
                Value::encode_reserved(&**self, output)
            }
            fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
                Ok(<$value as Value>::decode(input)?.into())
            }
        }
        impl crate::checkpoint::values::CheckpointValue for $name {
            const MINIMUM_BYTES: usize =
                <$value as crate::checkpoint::values::CheckpointValue>::MINIMUM_BYTES;
            fn encode(&self, output: &mut Vec<u8>) -> io::Result<()> {
                crate::checkpoint::values::CheckpointValue::encode(&**self, output)
            }
            fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
                <$value as crate::checkpoint::values::CheckpointValue>::decode(input)
                    .map(Into::into)
            }
        }
        opaque_value!($name);
    };
}
shared_market_value!(SharedMarketPool, MarketPool);
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct MarketPriceLevel {
    pub variant: u64,
    pub level: i32,
}
value_struct!(MarketPriceLevel {
    variant: u64,
    level: i32
});
impl OpaqueValue for MarketPriceLevel {}
shared_market_value!(SharedMarketPriceLevels, Vec<MarketPriceLevel>);
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct MarketSubsidy {
    pub cic_raw: i64,
    pub archetype: u32,
    pub kind: u8,
    pub targets: Vec<u32>,
    pub trigger: Vec<u8>,
}
value_struct!(MarketSubsidy { cic_raw: i64, archetype: u32, kind: u8, targets: Vec<u32>, trigger: Vec<u8> });
impl OpaqueValue for MarketSubsidy {}
shared_market_value!(SharedMarketSubsidies, Vec<MarketSubsidy>);

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct MarketVariable {
    pub name: Vec<u8>,
    pub amount_raw: i64,
}
value_struct!(MarketVariable {name:Vec<u8>,amount_raw:i64});
impl OpaqueValue for MarketVariable {}
shared_market_value!(SharedMarketVariables, Vec<MarketVariable>);
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct MarketContractRuntime {
    pub route_equipment: SharedMarketPool,
    pub route_length: i32,
    pub route_type: u8,
    pub route_request: i32,
    pub route_sunk_convoys_raw: i64,
    pub period_days: i32,
    pub meta_cic_raw: i64,
    pub meta_convoys_raw: i64,
    pub meta_efficiency_raw: i64,
    pub history_factory_raw: i64,
    pub history_equipment_raw: i64,
    pub history_collected_raw: i64,
}
value_struct!(MarketContractRuntime {
    route_equipment: SharedMarketPool,
    route_length: i32,
    route_type: u8,
    route_request: i32,
    route_sunk_convoys_raw: i64,
    period_days: i32,
    meta_cic_raw: i64,
    meta_convoys_raw: i64,
    meta_efficiency_raw: i64,
    history_factory_raw: i64,
    history_equipment_raw: i64,
    history_collected_raw: i64
});
macro_rules! borrowed_market_list {
    ($list:ty, $item:ty) => {
        impl<'a> IntoIterator for &'a $list {
            type Item = &'a $item;
            type IntoIter = std::slice::Iter<'a, $item>;
            fn into_iter(self) -> Self::IntoIter {
                self.iter()
            }
        }
    };
}
borrowed_market_list!(SharedMarketPriceLevels, MarketPriceLevel);
borrowed_market_list!(SharedMarketSubsidies, MarketSubsidy);
borrowed_market_list!(SharedMarketVariables, MarketVariable);