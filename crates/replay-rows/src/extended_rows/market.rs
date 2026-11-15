//! International market holdings and terms, and native lend-lease occurrences.
use super::*;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct MarketEquipment {
    pub variant: u64,
    pub amount_raw: i64,
}
value_struct!(MarketEquipment {
    variant: u64,
    amount_raw: i64
});
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct MarketPool {
    pub allow_zero: bool,
    pub items: Vec<MarketEquipment>,
}
value_struct!(MarketPool { allow_zero: bool, items: Vec<MarketEquipment> });
opaque_value!(MarketPool);

/// Owned market values shared by held rows, sparse updates and encoder baselines.
/// Only an actual write copies a value that still has another owner.
macro_rules! shared_market_value {
    ($name:ident, $value:ty) => {
        #[derive(Debug, Clone, PartialEq, Eq, replay_spool::Spooled)]
        pub struct $name(pub std::sync::Arc<$value>);
        impl Default for $name {
            fn default() -> Self {
                // Row codec key initializers must not allocate a fresh empty
                // collection before replacing it with a carried field.
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
#[derive(replay_spool::Spooled)]
pub struct MarketPriceLevel {
    pub variant: u64,
    pub level: i32,
}
value_struct!(MarketPriceLevel {
    variant: u64,
    level: i32
});
opaque_value!(Vec<MarketPriceLevel>);
shared_market_value!(SharedMarketPriceLevels, Vec<MarketPriceLevel>);
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct MarketSubsidy {
    pub cic_raw: i64,
    pub archetype: u32,
    pub kind: u8,
    pub targets: Vec<u32>,
    pub trigger: Vec<u8>,
}
value_struct!(MarketSubsidy { cic_raw: i64, archetype: u32, kind: u8, targets: Vec<u32>, trigger: Vec<u8> });
opaque_value!(Vec<MarketSubsidy>);
shared_market_value!(SharedMarketSubsidies, Vec<MarketSubsidy>);

/// Each domain is read only after its writer marks this holder. Unchanged
/// holders are carried under complete writer coverage, including empty holders.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct CountryMarketRow {
    pub country: u32,
    pub inventory: SharedMarketPool,
    pub prices: SharedMarketPriceLevels,
    pub subsidies: SharedMarketSubsidies,
    pub subsidy_total_raw: i64,
    /// Retired: the game 1.19.4 has no market-access auto-acceptance.
    pub auto_accept_market_access: RetiredBool,
    pub auto_send_market_access: bool,
    /// Retired: the game 1.19.4 has no purchase auto-acceptance.
    pub auto_accept_purchase: RetiredBool,
}
typed_codec!(CountryMarketRow, 32, |row| row.country as u128,
 |key| Self { country: key as u32, ..Default::default() },
 { 0 => (inventory): Opaque, 1 => (prices): Opaque, 2 => (subsidies): Opaque,
   3 => (subsidy_total_raw): I64, 4 => (auto_accept_market_access): Bool,
   5 => (auto_send_market_access): Bool, 6 => (auto_accept_purchase): Bool }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct MarketVariable {
    pub name: Vec<u8>,
    pub amount_raw: i64,
}
value_struct!(MarketVariable {name:Vec<u8>,amount_raw:i64});
opaque_value!(Vec<MarketVariable>);
shared_market_value!(SharedMarketVariables, Vec<MarketVariable>);
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
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

/// Kind 0 is a pending request, kind 1 an accepted purchase contract.
/// Ordered manifests, agreed prices and price levels are independent operands.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct MarketContractTermsRow {
    pub runtime: MarketContractRuntime,
    pub variables: SharedMarketVariables,
    pub variable_state: u64,
    pub country: u32,
    pub id: u64,
    pub kind: u8,
    pub buyer: u32,
    pub speed: i32,
    pub equipment: SharedMarketPool,
    pub prices: SharedMarketPool,
    pub price_levels: SharedMarketPriceLevels,
    pub subsidies: SharedMarketSubsidies,
}
typed_codec!(MarketContractTermsRow, 104,
 |row| (row.country as u128) << 72 | (row.id as u128) << 8 | row.kind as u128,
 |key| Self { country: (key >> 72) as u32, id: (key >> 8) as u64, kind: key as u8, ..Default::default() },
 { 0 => (buyer): U32, 1 => (speed): I32, 2 => (equipment): Opaque,
   3 => (prices): Opaque, 4 => (price_levels): Opaque, 5 => (subsidies): Opaque,
   6 => (runtime.route_equipment): Opaque, 7 => (runtime.route_length): I32,
   8 => (runtime.route_type): U8, 9 => (runtime.route_request): I32,
   10 => (runtime.route_sunk_convoys_raw): I64, 11 => (runtime.period_days): I32,
   12 => (runtime.meta_cic_raw): I64, 13 => (runtime.meta_convoys_raw): I64,
   14 => (runtime.meta_efficiency_raw): I64, 15 => (runtime.history_factory_raw): I64,
   16 => (runtime.history_equipment_raw): I64, 17 => (runtime.history_collected_raw): I64,
   18 => (variables): Opaque, 19 => (variable_state): U64 }, |row| if row.kind<=1 {Ok(())} else {Err(crate::codec::invalid("market terms kind unsupported"))});

/// Current agreement state. Saved cumulative statistics may saturate and reset
/// on an amendment; actual occurrence amounts remain in MarketEvents.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct LendLeaseAgreementRow {
    pub country: u32,
    pub id: u64,
    pub receiver: u32,
    /// Zero is the census baseline; observed amendments use occurrence + 1.
    pub revision: u64,
    pub active_since: i32,
    pub last_delivery: i32,
    pub monthly: SharedMarketPool,
    pub percentage: SharedMarketPool,
    pub once: SharedMarketPool,
    pub delivered: SharedMarketPool,
    pub sunk: SharedMarketPool,
    pub fuel_daily_raw: i64,
    pub fuel_percentage_raw: i64,
    pub fuel_sent_raw: i64,
    pub fuel_sunk_raw: i64,
    pub last_fuel_delivered_raw: i64,
}
typed_codec!(LendLeaseAgreementRow, 96,
 |row| (row.country as u128) << 64 | row.id as u128,
 |key| Self { country: (key >> 64) as u32, id: key as u64, ..Default::default() },
 { 0 => (receiver): U32, 1 => (revision): U64, 2 => (active_since): I32,
   3 => (last_delivery): I32, 4 => (monthly): Opaque, 5 => (percentage): Opaque,
   6 => (once): Opaque, 7 => (delivered): Opaque, 8 => (sunk): Opaque,
   9 => (fuel_daily_raw): I64, 10 => (fuel_percentage_raw): I64,
   11 => (fuel_sent_raw): I64, 12 => (fuel_sunk_raw): I64,
   13 => (last_fuel_delivered_raw): I64 }, |_row| Ok(()));

/// Market/lend-lease occurrences and changes in retained equipment totals.
/// Kind: 0 amendment, 2 ordered schedule entry, 3 increase in
/// delivered total, 4 increase in sunk total, 5 fuel store delta, 6 removal,
/// 10 request creation, 11 contract creation, 12 removal, 20 purchase delivery
/// total change, 21 payment. Equipment changes set flags bit 7; amount_raw is
/// the hour/revision difference and total_raw the reached cumulative value.
/// Rare final snapshots are differenced before amendment/removal; callback
/// admission pointers, holder marks and intermediate snapshots are not written.
/// Local draft-action resets are not recorded; schedules belong to agreements.
/// Kind 34 names a missing amount for this country/agreement identity: mode is
/// 3 delivered, 4 lost, or 20 purchase delivery; ordinal selects its refusal.
/// The occurrence row stream stays complete, including at encoding keyframes;
/// only the named amount is unavailable. Its zero payload is not an amount.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct MarketEventRow {
    pub id: u64,
    pub occurrence: u64,
    pub hour: i32,
    pub agreement: u64,
    pub country: u32,
    pub receiver: u32,
    pub kind: u8,
    pub mode: u8,
    pub ordinal: u32,
    pub variant: u64,
    pub amount_raw: i64,
    pub auxiliary_raw: i64,
    pub total_raw: i64,
    pub lost_total_raw: i64,
    pub date: i32,
    pub flags: u8,
}
impl MarketEventRow {
    /// A named refusal for this holder's missing delivery amount, never zero.
    pub fn delivery_unavailable_reason(&self) -> Option<&'static str> {
        if self.kind != 34 {
            return None;
        }
        Some(match self.ordinal {
            1 => "Delivery amount unavailable: required native cumulative total unreadable",
            2 => "Delivery amount unavailable: cumulative baseline uncertified",
            3 => "Delivery amount unavailable: final cumulative snapshot incomplete",
            4 => "Delivery amount unavailable: purchase manifest not acquired before removal",
            5 => "Delivery amount unavailable: holder erased without its final totals witness",
            _ => "Delivery amount unavailable: unsupported refusal reason",
        })
    }
}

typed_codec!(MarketEventRow, 64, |row| row.id as u128,
 |key| Self { id: key as u64, ..Default::default() },
 { 0 => (occurrence): U64, 1 => (hour): I32, 2 => (agreement): U64,
   3 => (country): U32, 4 => (receiver): U32, 5 => (kind): U8,
   6 => (mode): U8, 7 => (ordinal): U32, 8 => (variant): U64,
   9 => (amount_raw): I64, 10 => (auxiliary_raw): I64, 11 => (date): I32,
   12 => (flags): U8, 13 => (total_raw): I64, 14 => (lost_total_raw): I64 }, |row| if (row.kind<=30 && row.kind!=19) || row.kind==34 {Ok(())} else {Err(crate::codec::invalid("market event kind unsupported"))});

