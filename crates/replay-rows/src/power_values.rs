use super::*;

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
