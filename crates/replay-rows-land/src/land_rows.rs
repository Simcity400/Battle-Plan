use super::*;
use replay_rows::value::value_struct;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct LandSideLosses {
    pub manpower: i32,
    pub equipment: Vec<LandEquipmentLoss>,
}
value_struct!(LandSideLosses { manpower: i32, equipment: Vec<LandEquipmentLoss> });
impl OpaqueValue for LandSideLosses {}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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
impl OpaqueValue for LandTerminalObserver {}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DivisionManpowerContributor {
    pub country: u32,
    pub amount: i32,
}
value_struct!(DivisionManpowerContributor {
    country: u32,
    amount: i32
});
impl OpaqueValue for DivisionManpowerContributor {}
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DivisionLandInputsRow {
    pub division: u64,
    pub manpower: Vec<DivisionManpowerContributor>,
    pub received_supply: RetiredI64,
    pub maximum_grace: RetiredI64,
    pub remaining_grace: RetiredI64,
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
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct LandResultSide {
    pub countries: Vec<u32>,
    pub secondary_countries: Vec<u32>,
    pub commander: u64,
    pub outcome_flags: u8,
    pub progress: i64,
}
value_struct!(LandResultSide {countries:Vec<u32>,secondary_countries:Vec<u32>,commander:u64,outcome_flags:u8,progress:i64});
impl OpaqueValue for LandResultSide {}
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct LandCombatResultsRow {
    pub id: u64,
    pub combat: u64,
    pub province: u32,
    pub day: i32,
    pub duration: i32,
    pub end_hour: i32,
    pub sides: [LandResultSide; 2],
    pub winners: u8,
    pub border_war: bool,
    pub border_end_flags: [u8; 3],
    pub border_terminating: [u8; 2],
    pub on_action_winners: Option<u8>,
    pub history_countries: [u32; 2],
    pub history_tags: [Vec<u8>; 2],
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
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DestroyedDivisionsRow {
    pub id: u64,
    pub division: u64,
    pub owner: u32,
    pub killer_country: u32,
    pub killer_division: Option<u64>,
    pub province: u32,
    pub destination: Option<u32>,
    pub combats: Vec<u64>,
    pub hour: i32,
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
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct LandCombatEdgesRow {
    pub id: u64,
    pub kind: u8,
    pub hour: i32,
    pub combat: u64,
    pub division: Option<u64>,
    pub side: u8,
    pub manpower: Option<i32>,
    pub casualties_before: Option<i32>,
    pub casualties_after: Option<i32>,
    pub withheld: u8,
}
impl LandCombatEdgesRow {
    pub const OPENED: u8 = 0;
    pub const EXITED: u8 = 1;
    pub const NO_SIDE: u8 = u8::MAX;
    pub const COMBAT_UNREADABLE: u8 = 1;
    pub const DIVISION_UNREADABLE: u8 = 2;
    pub const SIDE_UNKNOWN: u8 = 4;
    pub const HOUR_UNREADABLE: u8 = 8;
    const REASONS: [(u8, &'static str); 4] = [
        (Self::COMBAT_UNREADABLE, "The combat could not be read"),
        (Self::DIVISION_UNREADABLE, "The division could not be read"),
        (Self::SIDE_UNKNOWN, "The division was on neither side of the combat"),
        (Self::HOUR_UNREADABLE, "The game hour could not be read; the row carries the hour it was drained"),
    ];

    pub fn withheld_reason(&self) -> Option<String> {
        let known = Self::REASONS.iter().fold(0, |bits, &(bit, _)| bits | bit);
        let mut reasons: Vec<&str> = Self::REASONS
            .iter()
            .filter(|&&(bit, _)| self.withheld & bit != 0)
            .map(|&(_, reason)| reason)
            .collect();
        if self.withheld & !known != 0 {
            reasons.push("Withheld for a reason this build does not know");
        }
        (!reasons.is_empty()).then(|| reasons.join("; "))
    }
}
typed_codec!(
    LandCombatEdgesRow,
    128,
    |r| (r.combat as u128) << 64 | r.id as u128,
    |k| Self { combat: (k >> 64) as u64, id: k as u64, ..Default::default() },
    {
        0 => (kind): U8,
        1 => (hour): I32,
        2 => (division): OptionalU64,
        3 => (side): U8,
        4 => (manpower): OptionalI32,
        5 => (casualties_before): OptionalI32,
        6 => (casualties_after): OptionalI32,
        7 => (withheld): U8
    },
    |r| {
        if r.kind > LandCombatEdgesRow::EXITED {
            Err(invalid("land combat edge kind invalid"))
        } else if r.side > 1 && r.side != LandCombatEdgesRow::NO_SIDE {
            Err(invalid("land combat edge side invalid"))
        } else {
            Ok(())
        }
    }
);

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct LandEventCoverageRow {
    pub id: u32,
    pub epoch: u64,
    pub from_hour: i32,
    pub to_hour: i32,
    pub complete: bool,
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
