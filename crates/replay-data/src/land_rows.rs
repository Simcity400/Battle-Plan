//! Land event observations and the hourly division companion columns.
use super::*;
use crate::value::value_struct;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LandEquipmentLoss {
    /// Full native equipment-variant handle; names resolve through recorded content.
    pub variant: u64,
    pub amount_raw: i64,
}
value_struct!(LandEquipmentLoss {
    variant: u64,
    amount_raw: i64
});
opaque_value!(Vec<LandEquipmentLoss>);

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LandSideLosses {
    pub manpower: i32,
    pub equipment: Vec<LandEquipmentLoss>,
}
value_struct!(LandSideLosses { manpower: i32, equipment: Vec<LandEquipmentLoss> });
opaque_value!([LandSideLosses; 2]);

/// Terminal supplemental observer operand; aggregate losses use `losses`.
/// Kinds have the same meanings as CombatLogObserverKind.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LandTerminalObserver {
    pub attacker: bool,
    pub kind: u8,
    pub item: u64,
    pub values: [u64; 5],
}
value_struct!(LandTerminalObserver {
    attacker: bool,
    kind: u8,
    item: u64,
    values: [u64; 5]
});
opaque_value!(Vec<LandTerminalObserver>);

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DivisionManpowerContributor {
    pub country: u32,
    pub amount: i32,
}
value_struct!(DivisionManpowerContributor {
    country: u32,
    amount: i32
});
opaque_value!(Vec<DivisionManpowerContributor>);
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DivisionLandInputsRow {
    pub division: u64,
    /// Raw contributor references and signed amounts, in native order.
    pub manpower: Vec<DivisionManpowerContributor>,
    /// Retired copies of `DivisionStatus.supply_ratio`, `supply_grace_maximum` and
    /// `supply_grace_remaining`: both tables took them from the same hour's DivisionSeed (the
    /// division's supply ratio and supply grace), so a division's DivisionStatus row is their
    /// record.
    pub received_supply: RetiredI64,
    pub maximum_grace: RetiredI64,
    pub remaining_grace: RetiredI64,
    /// The division's naval-transfer pointer is not null: the division is in a naval transfer.
    pub naval_transfer: bool,
}
typed_codec!(
    DivisionLandInputsRow,
    64,
    |r| r.division as u128,
    |k| Self { division: k as u64, ..Default::default() },
    {
        0 => (manpower): Opaque,
        1 => (received_supply): I64,
        2 => (maximum_grace): I64,
        3 => (remaining_grace): I64,
        4 => (naval_transfer): Bool
    },
    |_r| Ok(())
);

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LandResultSide {
    pub countries: Vec<u32>,
    pub secondary_countries: Vec<u32>,
    pub commander: u64,
    pub outcome_flags: u8,
    pub progress: i64,
}
value_struct!(LandResultSide {countries:Vec<u32>,secondary_countries:Vec<u32>,commander:u64,outcome_flags:u8,progress:i64});
opaque_value!([LandResultSide; 2]);
opaque_value!([u8; 3]);
opaque_value!([u8; 2]);
opaque_value!([Vec<u8>; 2]);
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LandCombatResultsRow {
    /// Hook sequence, unique within the attachment. Join on combat, not id.
    pub id: u64,
    pub combat: u64,
    pub province: u32,
    /// Round counter incremented after each non-terminating round. It is not
    /// elapsed days or a start date.
    pub day: i32,
    pub duration: i32,
    pub end_hour: i32,
    /// Attacker then defender. Same reference conventions as LandCombatSides.
    pub sides: [LandResultSide; 2],
    /// Combat finalization's winning branches: bit0 attacker, bit1 defender. Equal
    /// progress with both outcome bits clear awards the defender.
    pub winners: u8,
    pub border_war: bool,
    /// Border-war forced end flags and the two native participation predicates queried by its
    /// finalizer (255 means not queried).
    pub border_end_flags: [u8; 3],
    pub border_terminating: [u8; 2],
    /// Winner selected by round termination's leader on-actions, when dispatched.
    /// Participants are copied before its subsequent retreat.
    pub on_action_winners: Option<u8>,
    /// Exact post-finalizer end-history projection, distinct from pre-retreat
    /// `sides`. Zero denotes the native null country reference.
    pub history_countries: [u32; 2],
    /// Exact raw-tag strings used for saved history, independent of canonical
    /// country-link aliases or later country-slot reuse.
    pub history_tags: [Vec<u8>; 2],
    /// Attacker then defender, after combat finalization's pending-damage flush.
    pub losses: [LandSideLosses; 2],
    pub observers: Vec<LandTerminalObserver>,
}
typed_codec!(
    LandCombatResultsRow,
    64,
    |r| r.id as u128,
    |k| Self { id: k as u64, ..Default::default() },
    {
        0 => (combat): U64,
        1 => (province): U32,
        2 => (day): I32,
        3 => (duration): I32,
        4 => (end_hour): I32,
        5 => (sides): Opaque,
        6 => (winners): U8,
        7 => (border_war): Bool,
        8 => (border_end_flags): Opaque,
        9 => (border_terminating): Opaque,
        10 => (on_action_winners): Opaque,
        11 => (history_countries): Opaque,
        12 => (history_tags): Opaque,
        13 => (losses): Opaque,
        14 => (observers): Opaque
    },
    |r| {
        if r.winners > 3 { Err(invalid("land winner mask invalid")) } else { Ok(()) }
    }
);

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DestroyedDivisionsRow {
    pub id: u64,
    pub division: u64,
    pub owner: u32,
    pub killer_country: u32,
    /// Some(0) is a witnessed native null. None means the native death path
    /// selects no killer division; unavailable reads still fail the table.
    pub killer_division: Option<u64>,
    pub province: u32,
    /// Native destruction destination, absent when the wipe path selects none.
    pub destination: Option<u32>,
    /// Native combat references at destruction, in order. Empty is exact.
    pub combats: Vec<u64>,
    pub hour: i32,
    /// The wipe caller unlinks this combat before invoking participant lifecycle;
    /// keep it separate from the remaining native reference vector.
    pub triggering_combat: Option<u64>,
    pub combat_province: Option<u32>,
}
typed_codec!(
    DestroyedDivisionsRow,
    64,
    |r| r.id as u128,
    |k| Self { id: k as u64, ..Default::default() },
    {
        0 => (division): U64,
        1 => (owner): U32,
        2 => (killer_country): U32,
        3 => (killer_division): OptionalU64,
        4 => (province): U32,
        5 => (destination): OptionalU32,
        6 => (combats): U64List,
        7 => (hour): I32,
        8 => (triggering_combat): OptionalU64,
        9 => (combat_province): OptionalU32
    },
    |_r| Ok(())
);

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LandEventCoverageRow {
    pub id: u32,
    pub epoch: u64,
    pub from_hour: i32,
    pub to_hour: i32,
    /// Certifies the event interval independently of hourly casualty samples.
    pub complete: bool,
    /// Cumulative missed observations, including hourly casualty read shortages.
    pub lost_events: u64,
}
typed_codec!(
    LandEventCoverageRow,
    32,
    |r| r.id as u128,
    |k| Self { id: k as u32, ..Default::default() },
    {
        0 => (epoch): U64,
        1 => (from_hour): I32,
        2 => (to_hour): I32,
        3 => (complete): Bool,
        4 => (lost_events): U64
    },
    |_r| Ok(())
);
