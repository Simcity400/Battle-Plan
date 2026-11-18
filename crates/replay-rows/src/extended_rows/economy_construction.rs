use super::*;

/// What a production tick's applied input was for: the producer the tick's
/// credit returned to, and for military production whether the line was
/// converting stockpiled equipment into its design.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum OutputPurpose {
    /// The collector could not classify the tick, or the recording predates
    /// the purpose. Never read as new production.
    #[default]
    Unknown = 0,
    /// New equipment: military production that was not converting, naval
    /// construction or a railway gun.
    New = 1,
    /// Military production converting stockpiled equipment.
    Converted = 2,
    /// Ship refit production.
    Refit = 3,
}

impl OutputPurpose {
    pub const fn raw(self) -> u8 {
        self as u8
    }

    pub fn from_raw(raw: u8) -> Option<Self> {
        Some(match raw {
            0 => Self::Unknown,
            1 => Self::New,
            2 => Self::Converted,
            3 => Self::Refit,
            _ => return None,
        })
    }
}

/// Retained rocket totals per country/full design CID, independent of site removal.
/// Signed fixed5 daily production inputs and signed whole-unit deliveries
/// accumulate independently; ready counts are unrelated.
///
/// Variant-zero rows are world (country zero) or country coverage scopes, with
/// zero amounts. World exactness proves owner coverage; local flags also cover
/// unknown designs. Scope counters include designs and must never be summed with
/// them. Country zero on a design means an unknown producer, never a native slot.
/// Missing scope evidence stays unknown; a complete empty world proves no events.
/// Design rows retain known subtotals even when their total is inexact. Spending
/// and delivery failures, including overflow, never restore either exactness flag.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct RocketProductionOutputRow {
    pub country: u32,
    pub variant: u64,
    pub spent_raw: i64,
    pub instant_raw: i64,
    pub since_hour: i32,
    pub exact: bool,
    pub ticks: u64,
    pub inexact_ticks: u64,
    pub unknown_design_ticks: u64,
    pub unknown_country_ticks: u64,
    pub units: i64,
    pub units_exact: bool,
    pub deliveries: u64,
    pub inexact_deliveries: u64,
}
typed_codec!(RocketProductionOutputRow, 96, |row| (row.country as u128) << 64 | row.variant as u128, |key| Self { country: (key >> 64) as u32, variant: key as u64, ..Default::default() },
{ 0 => (spent_raw): I64, 1 => (instant_raw): I64, 2 => (since_hour): I32, 3 => (exact): Bool, 4 => (ticks): U64, 5 => (inexact_ticks): U64, 6 => (unknown_design_ticks): U64, 7 => (unknown_country_ticks): U64, 8 => (units): I64, 9 => (units_exact): Bool, 10 => (deliveries): U64, 11 => (inexact_deliveries): U64 }, |row| {
   if row.inexact_ticks > row.ticks
       || row.unknown_design_ticks > row.inexact_ticks
       || row.unknown_country_ticks > row.inexact_ticks
       || row.inexact_deliveries > row.deliveries
       || (row.variant == 0 && (row.spent_raw != 0 || row.instant_raw != 0 || row.units != 0))
       || (row.country != 0 && row.unknown_country_ticks != 0)
       || (row.variant != 0 && row.unknown_design_ticks != 0)
       || (row.country == 0 && row.exact && row.unknown_country_ticks != 0)
       || (row.country != 0 && row.exact && row.inexact_ticks != 0)
       || (row.country != 0 && row.units_exact && row.inexact_deliveries != 0)
       || (row.variant != 0 && row.ticks == 0 && row.deliveries == 0)
       || (row.variant != 0 && row.country == 0 && row.unknown_country_ticks != row.ticks)
       || (row.variant != 0 && row.country == 0
           && (row.inexact_deliveries != row.deliveries || (row.deliveries != 0 && row.units_exact)))
       || (row.ticks == 0 && (row.spent_raw != 0 || row.instant_raw != 0))
       || (row.deliveries == 0 && row.units != 0)
   {
       return Err(invalid("rocket output scope/counts inconsistent"));
   }
   Ok(())
});

#[cfg(test)]
#[path = "rocket_output/tests.rs"]
mod rocket_output_tests;

/// ConstructionLineOutput: key line (64 bits), 23 payload mask bits.
/// Branch totals from the observed daily advances, fixed5 since `since_hour`.
/// `construction_raw` and `repair_raw` are factory output, excluding `bank_raw`.
/// `instant_raw` is progress on the instant path; it has no factory
/// attribution (`exact` is false). `carry_raw` sums ordinary railway carries
/// for the progress self-check; carries are never additional spending.
/// `ticks` counts observed spend-site advances, including zero-IC building
/// advances; carry calls add no ticks. Linked primary output includes the
/// partner's factories, whose split is unknown. Its partner row is also linked,
/// with `exact` false: its amount is an unknown share, never an exact zero.
/// An ended line is published until a written frame held its final totals.
/// Lost observation withholds the table until a new branch.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ConstructionLineOutputRow {
    pub line: u64,
    pub railway: bool,
    pub construction_raw: i64,
    pub repair_raw: i64,
    pub bank_raw: i64,
    pub instant_raw: i64,
    pub carry_raw: i64,
    pub ticks: i32,
    /// Repair advances, including zero-IC natural repair.
    pub repair_ticks: i32,
    pub since_hour: i32,
    pub observed_since_creation: bool,
    pub linked: bool,
    pub ended: bool,
    pub exact: bool,
    /// None means no observed native completion decision since `since_hour`;
    /// NotRecorded is an older prefix. Independent of IC attribution/exactness.
    pub completion: Recorded<Option<ConstructionCompletion>>,
    /// Sum of the signed counts cleared on completion, independent of
    /// completed levels. Older publications retain their readable prefix.
    pub count_reset: Recorded<i64>,
    /// None when only a linked partner's unknown share was observed.
    pub country: Recorded<Option<u32>>,
    pub building_type: Recorded<Option<u32>>,
    pub target_flags: Recorded<Option<u32>>,
    pub target_location: Recorded<Option<u32>>,
    pub target_state: Recorded<Option<u32>>,
    pub target_country: Recorded<Option<u32>>,
    pub route_first: Recorded<Option<u32>>,
    pub route_last: Recorded<Option<u32>>,
}
typed_codec!(ConstructionLineOutputRow, 64, |row| row.line as u128, |key| Self { line: key as u64, ..Default::default() },
 { 0 => (railway): Bool, 1 => (construction_raw): I64, 2 => (repair_raw): I64, 3 => (bank_raw): I64, 4 => (instant_raw): I64, 5 => (carry_raw): I64, 6 => (ticks): I32, 7 => (since_hour): I32, 8 => (observed_since_creation): Bool, 9 => (linked): Bool, 10 => (ended): Bool, 11 => (exact): Bool, 12 => (repair_ticks): I32, 13 => (completion): Opaque, 14 => (count_reset): I64,
 15 => (country): OptionalU32, 16 => (building_type): OptionalU32, 17 => (target_flags): OptionalU32, 18 => (target_location): OptionalU32, 19 => (target_state): OptionalU32, 20 => (target_country): OptionalU32, 21 => (route_first): OptionalU32, 22 => (route_last): OptionalU32 }, |row| {
    if row.railway && matches!(row.count_reset, Recorded::Value(count) if count != 0) {
        return Err(invalid("railway output cannot contain a building count reset"));
    }
    if let Recorded::Value(Some(completion)) = row.completion
        && (!row.ended || completion.hour < row.since_hour || (completion.cancelled && !row.railway))
    {
        return Err(invalid("construction completion outcome inconsistent"));
    }
    Ok(())
});

/// A completion decision, observed before the line is deleted by daily
/// production or linked-partner removal during production advancement.
/// Railway completion checks also admit the cancel byte; that path is
/// removal rather than completion.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ConstructionCompletion {
    pub hour: i32,
    pub cancelled: bool,
}
value_struct!(ConstructionCompletion {
    hour: i32,
    cancelled: bool
});
opaque_value!(Option<ConstructionCompletion>);

/// VariantDefinitions: key id (64 bits), 14 payload mask bits. The equipment
/// type's name is content: `equipment_token` spelled by `LexicalTokens`.
///
/// `icon_active`, `icon_kind` and `icon` are the variant's raw icon entry: when active, kind 0
/// names a GFX sprite and kind 1 a texture path, and the kind-1 path `"."` asks the equipment
/// graphic pool instead.
///
/// The production row picks the icon from the variant and content in this
/// order; every sprite is looked up by name through
/// the content stack's interface files, and every country comes from the
/// variant, never from the viewer:
///
/// 1. An active entry that is not the path `"."`: a kind-0 sprite that exists,
///    or a kind-1 path whose file exists. Otherwise continue at step 3.
/// 2. An active `"."` (auto-designed variants): the graphic pool lookup below
///    for the creator country (`creator_link`), with its ruling party's
///    ideology group (the PoliticalParties row with `is_ruling`) and its
///    `graphical_culture` (CountryAppearance). Without a result, continue at
///    step 3.
/// 3. Let L be `origin_link` when positive, else the creator's original tag
///    when the creator is a dynamic country (CountryAppearance `name_tag`),
///    else the creator; `tag` is L's tag and S is `_medium`. A capital ship is
///    `classifier_flags` bit 6.
/// 4. For the equipment type T and then its archetype A:
///    `GFX_<tag>_<name><suffix>`, then `GFX_<name><suffix>`, with the suffix
///    `_capital` + S first for a capital ship, then S.
/// 5. When exactly one technology enables T (then A), for its id and its
///    alternate name n: `GFX_<tag>_<n>_medium` (the original tag when L is
///    dynamic), `GFX_<2D culture of L>_<n>_medium` (CountryAppearance
///    `graphical_culture_2d`), `GFX_<n>_medium`, capital-suffixed first for a
///    capital ship.
/// 6. T's `family`, when it differs from T: steps 4 to 8 for it.
/// 7. The graphic pool lookup for country L without ideology or culture.
/// 8. Otherwise the row shows no icon.
///
/// The graphic pool lookup reads `gfx/interface/equipmentdesigner/graphic_db`
/// in content order. It collects, for the country, its tag's root, then
/// `FACTION_<member>` for each member of its faction in order (FactionMembers),
/// then its capital's continent (Politics `original_capital` when nonzero, else
/// `capital`), then `default` (only `default` without a country); in each root,
/// the pools under T and then under its archetype, last declared first; and the
/// whole collection again for T's family when it differs. A pool counts when
/// its `limit` holds (vanilla limits test DLCs), it has no `sub_units` filter,
/// and its ideology and culture filters, when present, contain the key's. Pools
/// are stably sorted by `icons_weight` (or `weight` when that is zero), highest
/// first, and each pool's `icons` are tried last to first; the first existing
/// sprite or path wins.
///
/// The tag-string table's relation to the real tag, the capital-region key
/// (read as the continent), and the empty-result object's inactive state
/// remain unestablished.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct VariantDefinitionRow {
    pub id: u64,
    pub equipment_token: u32,
    pub creator_link: u32,
    pub origin_link: u32,
    pub append_variant_name: u8,
    pub obsolete: u8,
    pub classifier_flags: u64,
    pub mission_mask: u32,
    pub display_name: Vec<u8>,
    pub variant_name: Vec<u8>,
    /// The designer's saved model choice; empty when unset.
    pub explicit_entity: Vec<u8>,
    /// The definition's archetype lexer token.
    pub archetype_token: u32,
    pub icon_active: u8,
    pub icon_kind: u32,
    pub icon: Vec<u8>,
    /// Stored variant version byte. Older formats use NotRecorded; zero is
    /// a recorded version.
    pub version: Recorded<u8>,
    /// Stored ancestry, attribution, role and bonus state.
    pub design: Recorded<VariantDesignState>,
}
typed_codec!(VariantDefinitionRow, 64, |row| row.id as u128, |key| Self { id: key as u64, ..Default::default() },
 { 0 => (equipment_token): U32, 1 => (creator_link): U32, 2 => (origin_link): U32, 3 => (append_variant_name): U8, 4 => (obsolete): U8, 5 => (classifier_flags): U64, 6 => (mission_mask): U32, 7 => (display_name): Bytes, 8 => (variant_name): Bytes, 9 => (explicit_entity): Bytes, 10 => (archetype_token): U32, 11 => (icon_active): U8, 12 => (icon_kind): U32, 13 => (icon): Bytes, 14 => (version): U8, 15 => (design): Opaque }, |row| check_design_state(&row.design));

/// One requirement of a differing clone: archetype lexer token and raw fixed5 amount.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct LocalSubunitRequirement {
    pub archetype_token: u32,
    pub amount_raw: i64,
}
value_struct!(LocalSubunitRequirement {
    archetype_token: u32,
    amount_raw: i64
});

/// One target of a unit bonus: the sub-unit's or category's lexer token, the
/// signed fixed5 combat width, the nonzero dense stats in selector order and
/// the battalion grants in stored order.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct UnitBonusTarget {
    /// False for an exact sub-unit target, true for a sub-unit category target.
    pub category: bool,
    pub token: u32,
    pub combat_width_raw: i64,
    pub stats: Vec<UnitBonusStat>,
    pub grants: Vec<UnitBonusGrant>,
}
value_struct!(UnitBonusTarget {
    category: bool,
    token: u32,
    combat_width_raw: i64,
    stats: Vec<UnitBonusStat>,
    grants: Vec<UnitBonusGrant>
});

/// One battalion grant: the recipient category's lexer token, the raw add and
/// display-as-percentage bytes, and its sparse stats in stored order with
/// zero amounts and repeated selectors.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct UnitBonusGrant {
    pub category_token: u32,
    pub add: u8,
    pub display_as_percentage: u8,
    pub stats: Vec<UnitBonusStat>,
}
value_struct!(UnitBonusGrant {
    category_token: u32,
    add: u8,
    display_as_percentage: u8,
    stats: Vec<UnitBonusStat>
});

/// A stat selector, in the executable's stat order, and its raw fixed5 value.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct UnitBonusStat {
    pub selector: u32,
    pub value_raw: i64,
}
value_struct!(UnitBonusStat {
    selector: u32,
    value_raw: i64
});

/// One applied fuel receipt, as the collector observes each update: the
/// per-update form it keeps for a country's last 24 updates and the
/// recordings before [`FuelAwardDay`] stored.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FuelAwardRecipient {
    pub kind: u32,
    pub reference: u64,
    pub owner: u32,
    pub host: u64,
    pub amount_raw: i64,
    /// Pure native unit getter's retained text at the write, in game language.
    /// No air-wing name is guessed for a country/base container receipt.
    pub name: Option<Vec<u8>>,
    pub name_reason: Vec<u8>,
}
value_struct!(FuelAwardRecipient { kind: u32, reference: u64, owner: u32, host: u64,
    amount_raw: i64, name: Option<Vec<u8>>, name_reason: Vec<u8> });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FuelAwardHour {
    /// The settled end sample; update k belongs to (k-1,k].
    pub hour: i32,
    /// Fixed5: base, state-source, oil and negative-cost gain.
    pub gains_raw: [i64; 4],
    pub own_gain_raw: i64,
    /// Q15: incoming LL, outgoing LL, positive-cost debit and total other.
    pub transfers_raw: [i64; 4],
    pub consumed_raw: i64,
    pub received_raw: [i64; 3],
    pub recipients: Vec<FuelAwardRecipient>,
    /// Same-update source witness, before history clear. Its three sums are
    /// saved in the ring at history_index when remaining_hours was one.
    pub history_index: i32,
    pub remaining_hours: i32,
    pub pending_raw: [i64; 3],
    pub current_raw: i64,
}
value_struct!(FuelAwardHour { hour: i32, gains_raw: [i64; 4], own_gain_raw: i64,
    transfers_raw: [i64; 4], consumed_raw: i64, received_raw: [i64; 3],
    recipients: Vec<FuelAwardRecipient>, history_index: i32, remaining_hours: i32,
    pending_raw: [i64; 3], current_raw: i64 });

/// The 24 applied updates before a supplying read, update by update, as
/// recordings before [`FuelAwardDay`] stored them (`CountryFuel.awards`).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FuelAwardWindow {
    pub country_instance: u32,
    /// First authenticated update after an observed reset or continuity gap.
    /// Recorder evidence, not a native monotonic country identity.
    pub continuity_start_hour: Option<i32>,
    pub hours: Vec<FuelAwardHour>,
    /// Empty only when all 24 native updates in this read's window are known.
    /// A local refusal preserves the CountryFuel prefix and other countries.
    pub reason: Vec<u8>,
}
value_struct!(FuelAwardWindow { country_instance: u32, continuity_start_hour: Option<i32>, hours: Vec<FuelAwardHour>, reason: Vec<u8> });
opaque_value!(FuelAwardWindow);

/// One recipient's receipts over a [`FuelAwardDay`]: every receipt of the
/// day with the same native identity, owner, host and retained name and
/// name reason (each a separate recipient), what the Fuel widget's receipt
/// list and its history show.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct FuelAwardDayRecipient {
    pub kind: u32,
    pub reference: u64,
    pub owner: u32,
    pub host: u64,
    /// Pure native unit getter's retained text at the write, in game language.
    /// No air-wing name is guessed for a country/base container receipt.
    pub name: Option<Vec<u8>>,
    pub name_reason: Vec<u8>,
    /// The first and last update hours that paid it.
    pub first_hour: i32,
    pub last_hour: i32,
    /// Fixed5 sum of its receipts.
    pub amount_raw: i64,
}
value_struct!(FuelAwardDayRecipient { kind: u32, reference: u64, owner: u32, host: u64,
    name: Option<Vec<u8>>, name_reason: Vec<u8>, first_hour: i32, last_hour: i32,
    amount_raw: i64 });

/// The 24 applied fuel updates before a supplying read, summed as the Fuel
/// widget shows them: its day's sources, transfers, consumption and receipts
/// by consumer, the sum of each update's own gain converted to a quantity
/// (`engine_fuel::fuel_awards::quantity`, which no day sum can reproduce) and its
/// recipients. The collector sums the updates it observed; an update's
/// amounts and recipients, and the hourly source witness only the fresh-save
/// oracle compared, are not recorded.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct FuelAwardDay {
    pub country_instance: u32,
    /// First authenticated update after an observed reset or continuity gap.
    /// Recorder evidence, not a native monotonic country identity.
    pub continuity_start_hour: Option<i32>,
    /// The day's last update, the supplying read's own: the day is the
    /// updates `last_hour - 23 ..= last_hour`.
    pub last_hour: i32,
    /// Fixed5: base, state-source, oil and negative-cost gain.
    pub gains_raw: [i64; 4],
    pub own_gain_raw: i64,
    /// Quantity (/32768) units, each update's own gain converted.
    pub own_gain_quantity_raw: i64,
    /// Q15: incoming LL, outgoing LL, positive-cost debit and total other.
    pub transfers_raw: [i64; 4],
    pub consumed_raw: i64,
    /// Fixed5, by consumer kind.
    pub received_raw: [i64; 3],
    /// In order of kind, reference, owner, host, name and name reason.
    pub recipients: Vec<FuelAwardDayRecipient>,
    /// Empty only when all 24 native updates before this read are known and
    /// every sum fits its native i64; otherwise the sums and recipients are
    /// empty. A local refusal preserves the CountryFuel prefix and other
    /// countries.
    pub reason: Vec<u8>,
}
value_struct!(FuelAwardDay { country_instance: u32, continuity_start_hour: Option<i32>,
    last_hour: i32, gains_raw: [i64; 4], own_gain_raw: i64, own_gain_quantity_raw: i64,
    transfers_raw: [i64; 4], consumed_raw: i64, received_raw: [i64; 3],
    recipients: Vec<FuelAwardDayRecipient>, reason: Vec<u8> });
opaque_value!(FuelAwardDay);

/// CountryFuel: key country (32 bits), appended applied-award evidence.
///
/// One country's fuel-status scalars in their native representation: the quantities
/// (current, maximum and lend-lease gain) are i64 /32768.
///
/// The `ring_` fields are exact sums of the raw values of the game's 24 fuel
/// history ring slots: gain and received in fixed5, consumed and other in
/// /32768 units, received per consumer index. The game
/// normally writes one slot per hourly fuel update. A loaded non-default
/// countdown or prefilled ring does not prove 24 actual awards. The appended
/// `award_day` suffix sums 24 consecutive observed updates at this read and
/// keeps each recipient's receipts and stored name ([`FuelAwardDay`]); the
/// retired `awards` held the updates one by one. Unknown intervals preserve
/// the prefix and carry a reason, never zero. `CountryFuelConsumers` comes
/// from the same read of each fuel status.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CountryFuelRow {
    pub country: u32,
    pub current_raw: i64,
    pub maximum_raw: i64,
    /// Retired fields 2-4, 6-9 and 13-15: only the save comparison read them.
    pub gain_raw: RetiredI64,
    pub cost_raw: RetiredI64,
    pub gain_from_states_raw: RetiredI64,
    pub lend_lease_gain_raw: i64,
    pub lend_lease_consumption_raw: RetiredI64,
    pub gain_per_oil_raw: RetiredI64,
    pub history_index: RetiredI32,
    pub remaining_hours: RetiredI32,
    pub ring_gain_raw: i64,
    pub ring_consumed_raw: i64,
    pub ring_other_raw: i64,
    pub ring_requested_0_raw: RetiredI64,
    pub ring_requested_1_raw: RetiredI64,
    pub ring_requested_2_raw: RetiredI64,
    pub ring_received_0_raw: i64,
    pub ring_received_1_raw: i64,
    pub ring_received_2_raw: i64,
    pub country_instance: Recorded<Option<u32>>,
    /// Retired update-by-update window ([`FuelAwardWindow`]); `award_day`
    /// holds what the Fuel widget shows of it.
    pub awards: Recorded<Retired>,
    /// Current required-use rate operands, with their own supplying hour.
    pub demand: Recorded<FuelDemandRead>,
    /// The day's applied updates before this read, summed.
    pub award_day: Recorded<FuelAwardDay>,
}
typed_codec!(CountryFuelRow, 32, |row| row.country as u128, |key| Self { country: key as u32, ..Default::default() },
 { 0 => (current_raw): I64, 1 => (maximum_raw): I64, 2 => (gain_raw): I64, 3 => (cost_raw): I64, 4 => (gain_from_states_raw): I64, 5 => (lend_lease_gain_raw): I64, 6 => (lend_lease_consumption_raw): I64, 7 => (gain_per_oil_raw): I64, 8 => (history_index): I32, 9 => (remaining_hours): I32, 10 => (ring_gain_raw): I64, 11 => (ring_consumed_raw): I64, 12 => (ring_other_raw): I64, 13 => (ring_requested_0_raw): I64, 14 => (ring_requested_1_raw): I64, 15 => (ring_requested_2_raw): I64, 16 => (ring_received_0_raw): I64, 17 => (ring_received_1_raw): I64, 18 => (ring_received_2_raw): I64, 19 => (country_instance): OptionalU32, 20 => (awards): Opaque, 21 => (demand): Opaque, 22 => (award_day): Opaque }, |_row| Ok(()));

opaque_value!(Vec<LocalSubunitRequirement>);
opaque_value!(Vec<UnitBonusTarget>);

pub use game_model::equipment::{
    VariantBonusSource, VariantMissionStats, VariantModuleMissionExclusion, VariantNamedBonus,
    VariantStatEntry,
};
pub type VariantDesignState = game_model::equipment::VariantDesignState<u64>;
type NativeVariantDesignState = game_model::equipment::VariantDesignState;
value_struct!(VariantStatEntry {
    index: u32,
    value: i64
});
value_struct!(VariantModuleMissionExclusion {
    module_token: u32,
    mask: u32
});
value_struct!(VariantMissionStats { mask: u32, stats: Vec<VariantStatEntry> });
value_struct!(VariantNamedBonus { owner: u32, handle: u32, name: Vec<u8>, description: Vec<u8> });
value_struct!(VariantBonusSource { named: Option<VariantNamedBonus>, idea: Option<Vec<u8>>, idea_token: Option<u32>, stats: Vec<VariantStatEntry>, terrain_attrition: Vec<i64> });
macro_rules! design_value {
    ($state:ty, $sources:ty) => { value_struct!($state {
    parent: Option<u64>, max_version: u8, is_frame: u8, manpower: i32,
    auto_upgraded: u8, highlight: u8, can_upgrade_type: u8,
    can_upgrade_variant: u8, can_upgrade_modules: u8, role_icon_index: i32,
    division_names_group: Option<Vec<u8>>, pending_parent_version: i16,
    module_priority_sum: i32, module_mission_exclusions: Vec<VariantModuleMissionExclusion>,
    terrain_attrition: Vec<i64>, niches: Vec<u32>, build_cost: i64,
    build_resources: Vec<VariantStatEntry>, design_team: u64, design_team_serialized: bool, design_team_traits: i32,
    design_team_bonus: Vec<VariantStatEntry>, named_bonuses: Vec<u32>,
    bonus_sources: $sources, named_sources: $sources, mission_stats: Vec<VariantMissionStats>, unresolved_stats: Vec<VariantStatEntry>
    }); };
}
design_value!(VariantDesignState, Vec<u64>);
design_value!(NativeVariantDesignState, Vec<VariantBonusSource>);
opaque_value!(VariantDesignState);

fn check_design_state(value: &Recorded<VariantDesignState>) -> io::Result<()> {
    let Recorded::Value(state) = value else {
        return Ok(());
    };
    if state.design_team_serialized && state.design_team == 0 {
        return Err(invalid("resolved design team has a null reference"));
    }
    if state.bonus_sources.contains(&0) || state.named_sources.contains(&0) {
        return Err(invalid("design has a null bonus snapshot reference"));
    }
    for entry in state.mission_stats.iter().flat_map(|source| &source.stats) {
        if entry.index >= engine_reference_values::native_values::equipment_variant::STAT_COUNT as u32 {
            return Err(invalid("design stat index outside native metadata"));
        }
    }
    let expected = (0..engine_reference_values::native_values::equipment_variant::STAT_COUNT as u32).filter(|index| {
        ![0x2e, 0x2f, 0x30, 0x31, 0x3e, 0x41, 0x43, 0x44, 0x45, 0x4d].contains(index)
    });
    if (state.named_bonuses.is_empty() && !state.unresolved_stats.is_empty())
        || (!state.named_bonuses.is_empty()
            && !state
                .unresolved_stats
                .iter()
                .map(|entry| entry.index)
                .eq(expected))
    {
        return Err(invalid(
            "named bonus stat residuals are incomplete or unordered",
        ));
    }
    Ok(())
}
opaque_value!(VariantBonusSource);

/// Effective country operands requested by published equipment designs.
/// The key joins ModifierDefinitions; an absent native accumulator key is a
/// known zero. Failed owner/key reads retain a named scope and no value.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DesignCountryModifierRow {
    /// Raw creator-country reference, matching VariantDefinitions.creator_link.
    /// Several raw scopes may resolve to one native notification owner.
    pub country: u32,
    pub modifier: u32,
    pub value: Option<i64>,
    /// Zero: complete; one: country unavailable; two: modifier unavailable.
    pub failure: u8,
}
typed_codec!(DesignCountryModifierRow, 64,
|row| (row.country as u128) << 32 | row.modifier as u128,
|key| Self { country: (key >> 32) as u32, modifier: key as u32, ..Default::default() },
{ 0 => (value): OptionalI64, 1 => (failure): U8 }, |row| {
    if row.failure > 2 || (row.failure == 0) != row.value.is_some() {
        return Err(invalid("design country modifier availability inconsistent"));
    }
    Ok(())
});

opaque_value!(Vec<Vec<u32>>);
/// One demanded raw country scope. Independent columns keep consumption of a
/// use from rewriting unchanged factors and equipment rosters.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DesignCountryTemplatesRow {
    pub country: u32,
    pub uses: Vec<i32>,
    pub factors: Vec<i64>,
    pub equipment: Vec<Vec<u32>>,
    /// Zero: complete, including empty; one: owner unavailable; two: roster unreadable.
    pub failure: u8,
}
impl DesignCountryTemplatesRow {
    pub fn entries(&self) -> impl Iterator<Item = (i32, i64, &[u32])> {
        self.uses
            .iter()
            .copied()
            .zip(self.factors.iter().copied())
            .zip(&self.equipment)
            .map(|((uses, factor), equipment)| (uses, factor, equipment.as_slice()))
    }
}
typed_codec!(DesignCountryTemplatesRow, 32, |row| row.country as u128,
|key| Self { country: key as u32, ..Default::default() },
{ 0 => (uses): Opaque, 1 => (factors): Opaque, 2 => (equipment): Opaque, 3 => (failure): U8 }, |row| {
    if row.failure > 2 || row.uses.len() != row.factors.len()
        || row.uses.len() != row.equipment.len() || (row.failure != 0 && !row.uses.is_empty()) {
        return Err(invalid("design country template availability inconsistent"));
    }
    Ok(())
});

/// Stable value identity. A capture refuses a collision after comparing the
/// complete payload, so equal keys never stand in for unequal source values.
pub fn variant_bonus_snapshot_id(source: &VariantBonusSource) -> u64 {
    struct Digest(u64);
    impl Digest {
        fn bytes(&mut self, bytes: &[u8]) {
            for byte in bytes {
                self.0 = (self.0 ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3);
            }
        }
        fn word(&mut self, word: u64) {
            self.bytes(&word.to_le_bytes());
        }
    }
    let mut digest = Digest(0xcbf2_9ce4_8422_2325);
    match &source.named {
        None => digest.word(0),
        Some(named) => {
            digest.word(1);
            digest.word(u64::from(named.owner));
            digest.word(u64::from(named.handle));
            digest.word(named.name.len() as u64);
            digest.bytes(&named.name);
            digest.word(named.description.len() as u64);
            digest.bytes(&named.description);
        }
    }
    match &source.idea {
        None => digest.word(0),
        Some(text) => {
            digest.word(1);
            digest.word(text.len() as u64);
            digest.bytes(text);
        }
    }
    match source.idea_token {
        None => digest.word(0),
        Some(token) => {
            digest.word(1);
            digest.word(u64::from(token));
        }
    }
    digest.word(source.stats.len() as u64);
    for entry in &source.stats {
        digest.word(u64::from(entry.index));
        digest.word(entry.value as u64);
    }
    digest.word(source.terrain_attrition.len() as u64);
    for value in &source.terrain_attrition {
        digest.word(*value as u64);
    }
    digest.0.max(1)
}

/// A design state at a native mutation boundary. The copied definition is a
/// typed nested value so history survives deletion and repeated edits in an hour.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct VariantSnapshotDefinition {
    pub id: u64,
    pub equipment_token: u32,
    pub creator_link: u32,
    pub origin_link: u32,
    pub append_variant_name: u8,
    pub obsolete: u8,
    pub classifier_flags: u64,
    pub mission_mask: u32,
    pub display_name: Vec<u8>,
    pub variant_name: Vec<u8>,
    pub explicit_entity: Vec<u8>,
    pub archetype_token: u32,
    pub icon_active: u8,
    pub icon_kind: u32,
    pub icon: Vec<u8>,
    pub version: Recorded<u8>,
    pub design: Recorded<VariantDesignState>,
}
value_struct!(VariantSnapshotDefinition { id: u64, equipment_token: u32, creator_link: u32, origin_link: u32, append_variant_name: u8, obsolete: u8, classifier_flags: u64, mission_mask: u32, display_name: Vec<u8>, variant_name: Vec<u8>, explicit_entity: Vec<u8>, archetype_token: u32, icon_active: u8, icon_kind: u32, icon: Vec<u8>, version: Recorded<u8>, design: Recorded<VariantDesignState> });
impl From<VariantDefinitionRow> for VariantSnapshotDefinition {
    fn from(value: VariantDefinitionRow) -> Self {
        Self {
            id: value.id,
            equipment_token: value.equipment_token,
            creator_link: value.creator_link,
            origin_link: value.origin_link,
            append_variant_name: value.append_variant_name,
            obsolete: value.obsolete,
            classifier_flags: value.classifier_flags,
            mission_mask: value.mission_mask,
            display_name: value.display_name,
            variant_name: value.variant_name,
            explicit_entity: value.explicit_entity,
            archetype_token: value.archetype_token,
            icon_active: value.icon_active,
            icon_kind: value.icon_kind,
            icon: value.icon,
            version: value.version,
            design: value.design,
        }
    }
}
impl From<VariantSnapshotDefinition> for VariantDefinitionRow {
    fn from(value: VariantSnapshotDefinition) -> Self {
        Self {
            id: value.id,
            equipment_token: value.equipment_token,
            creator_link: value.creator_link,
            origin_link: value.origin_link,
            append_variant_name: value.append_variant_name,
            obsolete: value.obsolete,
            classifier_flags: value.classifier_flags,
            mission_mask: value.mission_mask,
            display_name: value.display_name,
            variant_name: value.variant_name,
            explicit_entity: value.explicit_entity,
            archetype_token: value.archetype_token,
            icon_active: value.icon_active,
            icon_kind: value.icon_kind,
            icon: value.icon,
            version: value.version,
            design: value.design,
        }
    }
}
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct VariantSnapshotModule {
    pub ordinal: u32,
    pub slot: u32,
    pub definition_token: u32,
    pub active: u8,
}
value_struct!(VariantSnapshotModule {
    ordinal: u32,
    slot: u32,
    definition_token: u32,
    active: u8
});
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct VariantSnapshotUpgrade {
    pub ordinal: u32,
    pub definition_token: i32,
    pub level: u8,
}
value_struct!(VariantSnapshotUpgrade {
    ordinal: u32,
    definition_token: i32,
    level: u8
});
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct VariantSnapshot {
    pub definition: VariantSnapshotDefinition,
    pub modules: Vec<VariantSnapshotModule>,
    pub stats: Vec<i64>,
    pub upgrades: Vec<VariantSnapshotUpgrade>,
}
value_struct!(VariantSnapshot { definition: VariantSnapshotDefinition, modules: Vec<VariantSnapshotModule>, stats: Vec<i64>, upgrades: Vec<VariantSnapshotUpgrade> });
opaque_value!(Option<VariantSnapshot>);

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct VariantSourceReferences {
    pub direct: Vec<u64>,
    pub named: Vec<u64>,
}
value_struct!(VariantSourceReferences { direct: Vec<u64>, named: Vec<u64> });
opaque_value!(Option<VariantSourceReferences>);

/// Independently current registry liveness for demanded raw design authors.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DesignOrganisationLivenessRow {
    pub organisation: u64,
    pub live: bool,
}
typed_codec!(DesignOrganisationLivenessRow, 64, |row| row.organisation as u128, |key| Self { organisation: key as u64, ..Default::default() },
    { 0 => (live): Bool }, |row| if row.organisation == 0 { Err(invalid("null design organisation liveness")) } else { Ok(()) });
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct VariantOrganisationChange {
    pub organisation: u64,
    pub live: bool,
}
value_struct!(VariantOrganisationChange {
    organisation: u64,
    live: bool
});
opaque_value!(Option<VariantOrganisationChange>);

/// A saved design edit that leaves its dependency population unchanged.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct VariantIconChange {
    pub active: u8,
    pub kind: u32,
    pub text: Vec<u8>,
}
value_struct!(VariantIconChange { active: u8, kind: u32, text: Vec<u8> });
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct VariantScalarChange {
    pub obsolete: Option<u8>,
    pub auto_upgraded: Option<u8>,
    pub highlight: Option<u8>,
    pub max_version: Option<u8>,
    pub role_icon_index: Option<i32>,
    pub display_name: Option<Vec<u8>>,
    pub model: Option<Vec<u8>>,
    pub icon: Option<VariantIconChange>,
    pub names_group: Option<Option<Vec<u8>>>,
    pub niches: Option<Vec<u32>>,
}
value_struct!(VariantScalarChange {
    obsolete: Option<u8>, auto_upgraded: Option<u8>, highlight: Option<u8>,
    max_version: Option<u8>, role_icon_index: Option<i32>, display_name: Option<Vec<u8>>,
    model: Option<Vec<u8>>, icon: Option<VariantIconChange>,
    names_group: Option<Option<Vec<u8>>>, niches: Option<Vec<u32>>
});
opaque_value!(Option<VariantScalarChange>);
impl VariantScalarChange {
    pub fn changes_design(&self) -> bool {
        self.auto_upgraded.is_some()
            || self.highlight.is_some()
            || self.max_version.is_some()
            || self.role_icon_index.is_some()
            || self.names_group.is_some()
            || self.niches.is_some()
    }
    pub fn apply_design(&self, state: &mut VariantDesignState) -> std::io::Result<()> {
        if let Some(value) = self.auto_upgraded {
            state.auto_upgraded = value;
        }
        if let Some(value) = self.highlight {
            state.highlight = value;
        }
        if let Some(value) = self.max_version {
            state.max_version = value;
        }
        if let Some(value) = self.role_icon_index {
            state.role_icon_index = value;
        }
        if let Some(value) = &self.names_group {
            state.division_names_group = crate::value::Value::try_clone(value)?;
        }
        if let Some(value) = &self.niches {
            state.niches = crate::value::Value::try_clone(value)?;
        }
        Ok(())
    }
    pub fn apply_definition(&self, row: &mut VariantDefinitionRow) -> std::io::Result<()> {
        if let Some(value) = self.obsolete {
            row.obsolete = value;
        }
        if let Some(value) = &self.display_name {
            row.display_name = crate::value::Value::try_clone(value)?;
        }
        if let Some(value) = &self.model {
            row.explicit_entity = crate::value::Value::try_clone(value)?;
        }
        if let Some(value) = &self.icon {
            row.icon_active = value.active;
            row.icon_kind = value.kind;
            row.icon = crate::value::Value::try_clone(&value.text)?;
        }
        if self.changes_design() {
            self.apply_design(
                row.design
                    .require_mut("VariantDefinitions.design")
                    .map_err(std::io::Error::other)?,
            )?;
        }
        Ok(())
    }
}

/// Ordered native changes since the preceding hourly boundary. Sequence zero
/// is reserved for a named observation failure; no missing interval proves that
/// a design stayed unchanged. Removal is an identity tombstone; preceding rows
/// retain the last readable design state.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct VariantChangeRow {
    pub sequence: u64,
    pub hour: i32,
    pub variant: u64,
    pub flags: Option<[u8; 3]>,
    pub scalar: Option<VariantScalarChange>,
    pub sources: Option<VariantSourceReferences>,
    pub organisation: Option<VariantOrganisationChange>,
    pub removed: bool,
    pub snapshot: Option<VariantSnapshot>,
    pub failure: Vec<u8>,
}
typed_codec!(VariantChangeRow, 64, |row| row.sequence as u128,
|key| Self { sequence: key as u64, ..Default::default() },
{ 0 => (hour): I32, 1 => (removed): Bool, 2 => (snapshot): Opaque, 3 => (failure): Bytes, 4 => (variant): U64, 5 => (flags): Opaque, 6 => (sources): Opaque, 7 => (organisation): Opaque, 8 => (scalar): Opaque }, |row| {
   if let Some(change) = &row.scalar {
       if row.sequence == 0 || row.variant == 0 || row.organisation.is_some() || row.snapshot.is_some() || row.flags.is_some() || row.sources.is_some() || row.removed || !row.failure.is_empty() || change == &VariantScalarChange::default() {
           return Err(invalid("invalid variant scalar change"));
       }
   } else if let Some(change) = &row.organisation {
       if row.sequence == 0 || row.variant != 0 || change.organisation == 0 || row.snapshot.is_some() || row.flags.is_some() || row.sources.is_some() || row.removed || !row.failure.is_empty() { return Err(invalid("invalid design organisation liveness change")); }
   } else if let Some(snapshot) = &row.snapshot {
       if row.sequence == 0 || row.variant != snapshot.definition.id || (row.flags.is_some() || row.sources.is_some()) || snapshot.stats.len() != 10 || !row.failure.is_empty() {
           return Err(invalid("invalid variant change snapshot"));
       }
       check_design_state(&snapshot.definition.design)?;
       if snapshot.modules.iter().enumerate().any(|(index, row)| row.ordinal as usize != index)
           || snapshot.upgrades.iter().enumerate().any(|(index, row)| row.ordinal as usize != index) {
           return Err(invalid("variant snapshot child ordinals are not contiguous"));
       }
   } else if row.sources.is_some() {
       if row.sequence == 0 || row.variant == 0 || row.flags.is_some() || row.removed || !row.failure.is_empty() { return Err(invalid("invalid variant source change")); }
   } else if row.flags.is_some() {
       if row.sequence == 0 || row.variant == 0 || row.removed || !row.failure.is_empty() { return Err(invalid("invalid variant flag change")); }
   } else if row.removed {
       if row.sequence == 0 || row.variant == 0 || !row.failure.is_empty() { return Err(invalid("invalid variant removal")); }
   } else if row.sequence != 0 || row.failure.is_empty() {
       return Err(invalid("variant change lacks its snapshot or observation failure"));
   }
   Ok(())
});

impl VariantSnapshot {
    pub fn try_definition(&self) -> std::io::Result<VariantDefinitionRow> {
        crate::value::Value::try_clone(&self.definition).map(Into::into)
    }
}

opaque_value!(Option<[u8; 3]>);

impl VariantSnapshotDefinition {
    pub fn matches(&self, row: &VariantDefinitionRow) -> bool {
        self.id == row.id
            && self.equipment_token == row.equipment_token
            && self.creator_link == row.creator_link
            && self.origin_link == row.origin_link
            && self.append_variant_name == row.append_variant_name
            && self.obsolete == row.obsolete
            && self.classifier_flags == row.classifier_flags
            && self.mission_mask == row.mission_mask
            && self.display_name == row.display_name
            && self.variant_name == row.variant_name
            && self.explicit_entity == row.explicit_entity
            && self.archetype_token == row.archetype_token
            && self.icon_active == row.icon_active
            && self.icon_kind == row.icon_kind
            && self.icon == row.icon
            && self.version == row.version
            && self.design == row.design
    }
}

#[cfg(test)]
mod design_contract_tests {
    use super::*;

    #[test]
    fn scalar_edits_round_trip_and_preserve_unwritten_fields() {
        let mut row = VariantChangeRow {
            sequence: 1,
            variant: 70,
            scalar: Some(VariantScalarChange {
                auto_upgraded: Some(1),
                names_group: Some(None),
                ..Default::default()
            }),
            ..Default::default()
        };
        row.check_key_fields().unwrap();
        let delta = row.delta(None).unwrap().unwrap();
        assert_eq!(VariantChangeRow::apply_delta(&delta, None).unwrap(), row);
        let mut definition = VariantDefinitionRow {
            design: Recorded::Value(VariantDesignState {
                manpower: 99,
                division_names_group: Some(b"old".to_vec()),
                ..Default::default()
            }),
            ..Default::default()
        };
        row.scalar
            .as_ref()
            .unwrap()
            .apply_definition(&mut definition)
            .unwrap();
        let state = definition.design.require("design").unwrap();
        assert_eq!(state.auto_upgraded, 1);
        assert_eq!(state.manpower, 99);
        assert_eq!(state.division_names_group, None);
        row.flags = Some([0; 3]);
        assert!(row.check_key_fields().is_err());
        row.flags = None;
        row.scalar = Some(Default::default());
        assert!(row.check_key_fields().is_err());
    }
    #[test]
    fn compact_sources_and_shared_liveness_have_separate_valid_row_contracts() {
        let mut source = VariantChangeRow {
            sequence: 1,
            variant: 7,
            sources: Some(VariantSourceReferences {
                direct: vec![8, 8],
                named: vec![9],
            }),
            ..Default::default()
        };
        source.check_key_fields().unwrap();
        let delta = source.delta(None).unwrap().unwrap();
        assert_eq!(VariantChangeRow::apply_delta(&delta, None).unwrap(), source);
        source.organisation = Some(VariantOrganisationChange {
            organisation: 99,
            live: false,
        });
        assert!(source.check_key_fields().is_err());
        let life = VariantChangeRow {
            sequence: 2,
            organisation: Some(VariantOrganisationChange {
                organisation: 99,
                live: false,
            }),
            ..Default::default()
        };
        life.check_key_fields().unwrap();
        let delta = life.delta(None).unwrap().unwrap();
        assert_eq!(VariantChangeRow::apply_delta(&delta, None).unwrap(), life);
        let row = DesignOrganisationLivenessRow {
            organisation: 99,
            live: false,
        };
        row.check_key_fields().unwrap();
    }

    #[test]
    fn variant_removal_is_an_identity_tombstone_without_a_snapshot() {
        let mut row = VariantChangeRow {
            sequence: 1,
            hour: 7,
            variant: u64::MAX,
            removed: true,
            ..Default::default()
        };
        row.check_key_fields().unwrap();
        let delta = row.delta(None).unwrap().unwrap();
        assert_eq!(VariantChangeRow::apply_delta(&delta, None).unwrap(), row);
        row.variant = 0;
        assert!(row.check_key_fields().is_err());
        row.variant = u64::MAX;
        row.removed = false;
        assert!(row.check_key_fields().is_err());
    }

    #[test]
    fn variant_country_modifier_availability_never_confuses_a_failure_with_zero() {
        let mut row = DesignCountryModifierRow {
            country: 5_000,
            modifier: u32::MAX,
            value: Some(0),
            failure: 0,
        };
        row.check_key_fields().unwrap();
        assert_eq!(row.key(), (5_000_u128 << 32) | u128::from(u32::MAX));
        row.value = None;
        assert!(row.check_key_fields().is_err());
        for failure in [1, 2] {
            row.failure = failure;
            row.check_key_fields().unwrap();
            row.value = Some(0);
            assert!(row.check_key_fields().is_err());
            row.value = None;
        }
        row.failure = 3;
        assert!(row.check_key_fields().is_err());
    }

    #[test]
    fn design_template_consumption_changes_only_uses_and_checks_parallel_rosters() {
        let previous = DesignCountryTemplatesRow {
            country: 5_000,
            uses: vec![2, -1],
            factors: vec![123_457, -1],
            equipment: vec![vec![8, 8], vec![]],
            failure: 0,
        };
        previous.check_key_fields().unwrap();
        let mut next = previous.clone();
        next.uses[0] -= 1;
        let delta = next.delta(Some(&previous)).unwrap().unwrap();
        assert_eq!(delta.mask, 1);
        assert_eq!(
            DesignCountryTemplatesRow::apply_delta(&delta, Some(previous)).unwrap(),
            next
        );
        next.factors.pop();
        assert!(next.check_key_fields().is_err());
        let mut empty = DesignCountryTemplatesRow::default();
        empty.check_key_fields().unwrap();
        empty.failure = 2;
        empty.check_key_fields().unwrap();
        empty.uses.push(0);
        assert!(empty.check_key_fields().is_err());
    }

    #[test]
    fn variant_design_contract_refuses_bad_stat_residuals_and_null_live_team() {
        let mut row = VariantDefinitionRow {
            id: 70,
            design: Recorded::Value(VariantDesignState::default()),
            ..Default::default()
        };
        row.check_key_fields().unwrap();
        row.design
            .require_mut("design")
            .unwrap()
            .design_team_serialized = true;
        assert!(row.check_key_fields().is_err());
        let state = row.design.require_mut("design").unwrap();
        state.design_team_serialized = false;
        state.design_team = u64::MAX; // An unresolved raw reference is retained.
        state.named_bonuses.push(9);
        assert!(row.check_key_fields().is_err());
        let state = row.design.require_mut("design").unwrap();
        state.unresolved_stats = (0..engine_reference_values::native_values::equipment_variant::STAT_COUNT as u32)
            .filter(|index| {
                ![0x2e, 0x2f, 0x30, 0x31, 0x3e, 0x41, 0x43, 0x44, 0x45, 0x4d].contains(index)
            })
            .map(|index| VariantStatEntry {
                index,
                value: i64::MIN,
            })
            .collect();
        row.check_key_fields().unwrap();
        row.design.require_mut("design").unwrap().unresolved_stats[0].index = 1;
        assert!(row.check_key_fields().is_err());
    }
}