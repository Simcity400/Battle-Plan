use super::*;
use crate::value::value_struct;

macro_rules! politics_kind {
    ($name:ident, $ty:ident, $kind:ident, $($variant:ident = $value:literal),+ $(,)?) => {
        #[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
        #[repr($ty)]
        pub enum $name { #[default] $($variant = $value),+ }
        impl NativeValue for $name {
            fn heap_bytes(&self) -> usize { 0 }
            fn try_clone(&self) -> io::Result<Self> { Ok(*self) }
            fn field(&self) -> Field<'_> { Field::$kind(*self as $ty) }
            fn from_field(field: Field<'_>) -> io::Result<Self> {
                let Field::$kind(value) = field else { return Err(invalid("politics kind has wrong wire type")); };
                match value { $($value => Ok(Self::$variant),)+ _ => Err(invalid(concat!("unknown ", stringify!($name)))) }
            }
        }
        impl TryFrom<$ty> for $name {
            type Error = io::Error;
            fn try_from(value: $ty) -> io::Result<Self> { Self::from_field(Field::$kind(value)) }
        }
        impl Value for $name {
            fn heap_bytes(&self) -> usize { 0 }
            const MINIMUM_ENCODED_BYTES: usize = size_of::<$ty>();
            fn encoded_bytes(&self) -> io::Result<usize> { Ok(size_of::<$ty>()) }
            fn try_clone(&self) -> io::Result<Self> { Ok(*self) }
            fn encode_reserved(&self, output: &mut Vec<u8>) -> io::Result<()> { <$ty as Value>::encode_reserved(&(*self as $ty), output) }
            fn decode(input: &mut Decoder<'_>) -> io::Result<Self> { Self::from_field(Field::$kind(<$ty as Value>::decode(input)?)) }
        }
    };
}
politics_kind!(PowerCurrency, u8, U8, Political = 0, Command = 1);
politics_kind!(
    PowerOperation,
    u8,
    U8,
    Add = 0,
    Subtract = 1,
    Set = 2,
    Reset = 3,
    AllocationClamp = 4,
    Clamp = 5,
    Grant = 6,
    Refund = 7
);
politics_kind!(
    PowerCause,
    u16,
    U16,
    NotIdentified = 0,
    DailyGain = 1,
    Idea = 2,
    Advisor = 3,
    AdvisorRemoval = 4,
    Decision = 5,
    TargetedDecision = 6,
    DecisionTimeout = 7,
    Diplomacy = 8,
    WarJustification = 9,
    JoinWar = 10,
    Focus = 11,
    FocusRefund = 12,
    ScriptEffect = 13,
    ScaledScriptEffect = 14,
    Autonomy = 15,
    SpyMaster = 16,
    ScientistRecruitment = 17,
    ScientistFaction = 18,
    ScorchedEarth = 19,
    PrideOfFleet = 20,
    Blockade = 21,
    MioPolicy = 22,
    Medal = 23,
    Console = 24,
    ResetSetup = 25,
    AbilityActivation = 26,
    AbilityUpkeep = 27,
    Allocation = 28,
    AllocationRefund = 29,
    LeaderTrait = 30,
    LeaderPromotion = 31,
    LeaderAssignment = 32,
    LeaderRecruitment = 33,
    Raid = 34,
    AirAction = 35,
    PreferredTactic = 36,
    OfficerUpgrade = 37,
    CaptainUpgrade = 38,
    TimedActivity = 39,
    CapCorrection = 40
);
politics_kind!(
    PowerSite,
    u16,
    U16,
    NotIdentified = 0,
    AddPoliticalPower = 1,
    AddPoliticalPowerNoDeficit = 2,
    SetPoliticalPower = 3,
    ClearPolitics = 4,
    ResetCountry = 5,
    AddCommandPower = 6,
    SpendCommandPower = 7,
    AllocateCommandPower = 8,
    ReleaseCommandPower = 9,
    ActivateAbility = 10,
    AbilityUpkeep = 11,
    GroundCrew = 12,
    SetRulingParty = 13,
    ForceRulingParty = 14,
    SetupPolitics = 15,
    FallbackRulingParty = 16
);
politics_kind!(
    PowerObjectKind,
    u8,
    U8,
    None = 0,
    Leader = 1,
    OrdersGroup = 2,
    Ship = 3,
    Raid = 4,
    Organisation = 5,
    AirWing = 6,
    Ability = 7
);
politics_kind!(
    PowerOriginKind,
    u8,
    U8,
    None = 0,
    Decision = 1,
    Focus = 2,
    Event = 3,
    OnAction = 4,
    Console = 5,
    NativeCommand = 6,
    ScriptEffect = 7
);
politics_kind!(
    PowerDailyCostKind,
    u8,
    U8,
    PoliticalPowerCostModifier = 0,
    Relation = 1,
    MioResearch = 2,
    MioProduction = 3,
    Blockade = 4
);
politics_kind!(
    GovernmentCause,
    u8,
    U8,
    NotIdentified = 0,
    ElectionOnAction = 1,
    CoupCompletion = 2,
    CivilWar = 3,
    SetPoliticsEffect = 4,
    PeaceForceGovernment = 5,
    SubjectGovernment = 6,
    Console = 7,
    ResetSetup = 8
);

politics_kind!(
    PowerAction,
    u8,
    U8,
    None = 0,
    Add = 1,
    Remove = 2,
    Replace = 3,
    Execute = 4,
    Timeout = 5,
    Promote = 6,
    LearnTrait = 7,
    Activate = 8,
    Upkeep = 9,
    Start = 10,
    Stop = 11
);

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct PowerIdentity {
    pub action: PowerAction,
    pub item_token: Option<u32>,
    pub item_key: Vec<u8>,
    pub slot_key: Vec<u8>,
    pub character: Option<u32>,
    pub object_kind: PowerObjectKind,
    pub object_reference: Option<u64>,
    pub target_country: Option<u32>,
    pub target_state: Option<u32>,
    pub target_region: Option<u32>,
    pub origin_kind: PowerOriginKind,
    pub origin_token: Option<u32>,
    pub origin_key: Vec<u8>,
}
value_struct!(PowerIdentity {
    action: PowerAction,
    item_token: Option<u32>, item_key: Vec<u8>, slot_key: Vec<u8>,
    character: Option<u32>, object_kind: PowerObjectKind, object_reference: Option<u64>,
    target_country: Option<u32>, target_state: Option<u32>, target_region: Option<u32>,
    origin_kind: PowerOriginKind, origin_token: Option<u32>, origin_key: Vec<u8>
});
opaque_value!(PowerIdentity);
impl PowerIdentity {
    fn check(&self) -> io::Result<()> {
        if (self.object_kind == PowerObjectKind::None) != self.object_reference.is_none() {
            return Err(invalid("power reference requires its domain"));
        }
        if self.origin_kind == PowerOriginKind::None
            && (self.origin_token.is_some() || !self.origin_key.is_empty())
        {
            return Err(invalid("power origin identity requires its kind"));
        }
        Ok(())
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CountryPowerDailyRow {
    pub country: u32,
    pub instance: u32,
    pub epoch: u64,
    pub day_hour: i32,
    pub pp_gross_gain_raw: Option<i64>,
    pub pp_cost_raw: Option<i64>,
    pub pp_net_gain_raw: Option<i64>,
    pub pp_realized_delta_raw: Option<i64>,
    pub pp_sequence: Option<u64>,
    pub cp_gain_raw: Option<i64>,
    pub cp_realized_delta_raw: Option<i64>,
    pub cp_sequence: Option<u64>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CountryPowerChangeRow {
    pub country: u32,
    pub instance: u32,
    pub sequence: u64,
    pub epoch: u64,
    pub hour: i32,
    pub currency: PowerCurrency,
    pub operation: PowerOperation,
    pub balance_before_raw: i64,
    pub balance_after_raw: i64,
    pub requested_delta_raw: Option<i64>,
    pub cause: PowerCause,
    pub site: PowerSite,
    pub identity: PowerIdentity,
}
impl CountryPowerDailyRow {
    fn valid_rates(&self) -> bool {
        let pp = match (
            self.pp_gross_gain_raw,
            self.pp_cost_raw,
            self.pp_net_gain_raw,
            self.pp_realized_delta_raw,
            self.pp_sequence,
        ) {
            (Some(gross), Some(cost), Some(net), Some(_), Some(_)) => {
                gross.wrapping_sub(cost) == net
            }
            (None, None, None, None, None) => true,
            _ => false,
        };
        pp && self.cp_gain_raw.is_some() == self.cp_realized_delta_raw.is_some()
            && self.cp_gain_raw.is_some() == self.cp_sequence.is_some()
    }
}
impl CountryPowerChangeRow {
    pub fn realized_delta_raw(&self) -> i128 {
        i128::from(self.balance_after_raw) - i128::from(self.balance_before_raw)
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CountryPowerDailyCostRow {
    pub country: u32,
    pub instance: u32,
    pub ordinal: u32,
    pub epoch: u64,
    pub day_hour: i32,
    pub pp_sequence: u64,
    pub component: PowerDailyCostKind,
    pub cost_raw: i64,
    pub cause: PowerCause,
    pub site: PowerSite,
    pub identity: PowerIdentity,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct GovernmentChangeRow {
    pub country: u32,
    pub instance: u32,
    pub sequence: u64,
    pub epoch: u64,
    pub hour: i32,
    pub before_party: Option<u32>,
    pub after_party: Option<u32>,
    pub before_ideology: Option<u32>,
    pub after_ideology: Option<u32>,
    pub cause: GovernmentCause,
    pub actor_country: Option<u32>,
    pub origin_kind: PowerOriginKind,
    pub origin_token: Option<u32>,
    pub origin_key: Vec<u8>,
    pub site: PowerSite,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct PoliticsCaptureCoverageRow {
    pub country: u32,
    pub instance: u32,
    pub epoch: u64,
    pub since_hour: i32,
    pub through_hour: i32,
    pub streams: u8,
}
impl PoliticsCaptureCoverageRow {
    pub const PP: u8 = 1;
    pub const CP: u8 = 2;
    pub const PP_DAILY_COSTS: u8 = 4;
    pub const GOVERNMENT: u8 = 8;
    pub const ALL_STREAMS: u8 = Self::PP | Self::CP | Self::PP_DAILY_COSTS | Self::GOVERNMENT;
}

typed_codec!(CountryPowerDailyRow, 32, |row| row.country as u128, |key| Self { country: key as u32, ..Self::default() },
 { 0 => (epoch): U64, 1 => (day_hour): I32, 2 => (pp_gross_gain_raw): OptionalI64, 3 => (pp_cost_raw): OptionalI64, 4 => (pp_net_gain_raw): OptionalI64, 5 => (pp_realized_delta_raw): OptionalI64, 6 => (pp_sequence): OptionalU64, 7 => (cp_gain_raw): OptionalI64, 8 => (cp_realized_delta_raw): OptionalI64, 9 => (cp_sequence): OptionalU64 , 10 => (instance): U32 }, |row| if row.day_hour.rem_euclid(24) != 0 || !row.valid_rates() { Err(invalid("invalid daily power origin or sum")) } else { Ok(()) });

typed_codec!(CountryPowerChangeRow, 96, |row| (row.country as u128) << 64 | row.sequence as u128, |key| Self { country: (key >> 64) as u32, sequence: key as u64, ..Self::default() },
 { 0 => (epoch): U64, 1 => (hour): I32, 2 => (currency): U8, 3 => (operation): U8, 4 => (balance_before_raw): I64, 5 => (balance_after_raw): I64, 6 => (requested_delta_raw): OptionalI64, 7 => (cause): U16, 8 => (site): U16, 9 => (identity): Opaque , 10 => (instance): U32 }, |row| row.identity.check());

typed_codec!(CountryPowerDailyCostRow, 64, |row| (row.country as u128) << 32 | row.ordinal as u128, |key| Self { country: (key >> 32) as u32, ordinal: key as u32, ..Self::default() },
 { 0 => (epoch): U64, 1 => (day_hour): I32, 2 => (pp_sequence): U64, 3 => (component): U8, 4 => (cost_raw): I64, 5 => (cause): U16, 6 => (site): U16, 7 => (identity): Opaque , 8 => (instance): U32 }, |row| if row.day_hour.rem_euclid(24) != 0 { Err(invalid("daily cost must retain midnight origin")) } else { row.identity.check() });

typed_codec!(GovernmentChangeRow, 96, |row| (row.country as u128) << 64 | row.sequence as u128, |key| Self { country: (key >> 64) as u32, sequence: key as u64, ..Self::default() },
 { 0 => (epoch): U64, 1 => (hour): I32, 2 => (before_party): OptionalU32, 3 => (after_party): OptionalU32, 4 => (before_ideology): OptionalU32, 5 => (after_ideology): OptionalU32, 6 => (cause): U8, 7 => (actor_country): OptionalU32, 8 => (origin_kind): U8, 9 => (origin_token): OptionalU32, 10 => (origin_key): Bytes, 11 => (site): U16 , 12 => (instance): U32 }, |row| if row.origin_kind == PowerOriginKind::None && (row.origin_token.is_some() || !row.origin_key.is_empty()) { Err(invalid("government origin identity requires its kind")) } else { Ok(()) });

typed_codec!(PoliticsCaptureCoverageRow, 32, |row| row.country as u128, |key| Self { country: key as u32, ..Self::default() },
 { 0 => (epoch): U64, 1 => (since_hour): I32, 2 => (through_hour): I32, 3 => (streams): U8 , 4 => (instance): U32 }, |row| if row.since_hour > row.through_hour || row.streams & !Self::ALL_STREAMS != 0 { Err(invalid("invalid politics coverage interval or bits")) } else { Ok(()) });

