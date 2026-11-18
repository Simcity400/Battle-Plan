use super::*;

fn multiply(a: i64, b: i64, denominator: i64) -> Option<i64> {
    a.wrapping_mul(b).checked_div(denominator)
}
/// ProductionLines: key handle (64 bits), 13 payload mask bits.
///
/// `produced_raw` is progress toward the next unit (fixed5), not a count of units produced.
/// `efficiency_raw` is the current efficiency the production view shows, i64 Q15 (x32768, so 100%
/// is 3276800): the game's average of the first min(`active_factories`, `max_factories`)
/// per-factory efficiencies, truncated as `game_model::production::efficiency_average`
/// computes it; the row shows it rounded to a whole percent, `(efficiency_raw + (1 << 14)) >> 15`.
/// `max_factories` is the line's maximum factories, the count that average is clamped to; the
/// per-factory values are retained in ProductionLineModifiers.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ProductionLineRow {
    pub handle: u64,
    pub country: u32,
    pub kind: u8,
    pub variant: u64,
    pub mio: u64,
    pub active_factories: i32,
    pub requested_factories: i32,
    pub cost_raw: i64,
    pub speed_raw: i64,
    /// Retired: only the save comparison and its self-check read it.
    pub produced_raw: RetiredI64,
    pub priority: i32,
    pub amount: i32,
    pub max_factories: i32,
    pub efficiency_raw: i64,
}

impl ProductionLineRow {
    /// The matching recorded factory prefix's signed native average. The caller
    /// separately admits the supplying table's completeness. Identity and count
    /// disagreement, or an incomplete prefix, supplies no candidate.
    pub fn factory_efficiency(&self, source: &ProductionLineModifiersRow) -> Option<i64> {
        if (
            self.handle,
            self.country,
            self.variant,
            self.mio,
            self.kind,
            self.active_factories,
        ) != (
            source.line,
            source.country,
            source.variant,
            source.organisation,
            source.kind,
            source.active_factories,
        ) {
            return None;
        }
        let count = self.active_factories.min(self.max_factories).max(0);
        let values = source
            .factory_efficiencies
            .get(..usize::try_from(count).ok()?)?;
        let sum = values
            .iter()
            .fold(0_u64, |sum, &value| sum.wrapping_add(value as u64));
        Some(game_model::production::efficiency_average(sum, count))
    }
}
typed_codec!(ProductionLineRow, 64, |row| row.handle as u128, |key| Self { handle: key as u64, ..Default::default() },
 { 0 => (country): U32, 1 => (kind): U8, 2 => (variant): U64, 3 => (mio): U64, 4 => (active_factories): I32, 5 => (requested_factories): I32, 6 => (cost_raw): I64, 7 => (speed_raw): I64, 8 => (produced_raw): I64, 9 => (priority): I32, 10 => (amount): I32, 11 => (max_factories): I32, 12 => (efficiency_raw): I64 }, |_row| Ok(()));

/// IndustrialOrganisations: key organisation (64 bits), 6 payload mask bits.
///
/// One military-industrial organisation, keyed by its full CID (type 79 low, id high),
/// the reference `ProductionLines.mio` holds. `country` is the raw reference of the
/// country whose production status holds it and `ordinal` its position there. `owner` is
/// the tag link the organisation was constructed with, the tag its name and icon rules
/// prefix. `token` is the script key, spelled by `LexicalTokens`. `name` is the name-key
/// override and `icon` the sprite name, both empty when unset. Display localises the
/// override, or the script key when empty, preferring `<owner tag>_<that key>` when
/// localisation has it; the text may use scripted localisation in the organisation's
/// scope. The icon uses the stored sprite, or `GFX_<script key>` (a tag-prefixed
/// candidate first), or `GFX_idea_unknown`. Read with `IndustrialOrganisationHistory` in
/// the lines' daily slot.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct IndustrialOrganisationRow {
    pub organisation: u64,
    pub country: u32,
    pub ordinal: u32,
    pub owner: u32,
    pub token: u32,
    pub name: Vec<u8>,
    pub icon: Vec<u8>,
}
typed_codec!(IndustrialOrganisationRow, 64, |row| row.organisation as u128, |key| Self { organisation: key as u64, ..Default::default() },
 { 0 => (country): U32, 1 => (ordinal): U32, 2 => (owner): U32, 3 => (token): U32, 4 => (name): Bytes, 5 => (icon): Bytes }, |_row| Ok(()));

/// IndustrialOrganisationHistory: key organisation:variant (128 bits), 1
/// payload mask bit.
///
/// The units of one equipment variant that production lines assigned to the
/// organisation have completed, as the game counts them for its history: one
/// occupied slot of the organisation's history table. The game adds each
/// production tick's completed units and never resets the count; a line with
/// no organisation adds to none, and a naval line that cannot deploy still
/// adds its count. It is the only exact count of produced units the game keeps
/// below the country; there is none per line.
/// Read with `IndustrialOrganisations` in the lines' daily slot.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct IndustrialOrganisationHistoryRow {
    pub organisation: u64,
    pub variant: u64,
    pub units: i32,
    pub design_hour: Recorded<Option<i32>>,
}
typed_codec!(IndustrialOrganisationHistoryRow, 128, |row| (row.organisation as u128) << 64 | row.variant as u128, |key| Self { organisation: (key >> 64) as u64, variant: key as u64, ..Default::default() },
 { 0 => (units): I32, 1 => (design_hour): OptionalI32 }, |_row| Ok(()));

/// ProductionLineOutput: key line:ordinal (96 bits), 12 payload mask bits.
///
/// The units one production line has completed, which the game counts nowhere
/// per line: the collector observes every production tick (its
/// `line_output` hooks) and totals them per recording branch. One row per
/// (country, variant, organisation, purpose) tuple the line produced under,
/// `ordinal` being its first-seen position on the branch (a replay key holds
/// at most 128 bits, so the tuple is payload); a line's total is the sum of
/// its rows. A line that switches between new production and conversion
/// therefore keeps each purpose's earlier input in its own row.
/// `line` is `ProductionLines.handle`, `variant` a variant CID and `mio` the
/// organisation CID (`ProductionLines.mio`, zero for none). `country` is the
/// producing country's raw reference read from the line's production status
/// at the tick, resolved through CountryLinks even when a line without an
/// organisation completes and disappears between censuses. The variant's
/// creator does not supply it; zero rows seeded by a census use that census's
/// country.
///
/// `units` are the units delivered since `since_hour`, the sample that began
/// the branch's observation. `mio_credit` is what the game credited the
/// organisation over the same interval (`IndustrialOrganisationHistory`); the
/// two differ exactly when a naval line that cannot deploy for lack of
/// manpower still credits its count, or a tick passes a non-positive count.
/// `observed_since_creation` says the line was first seen after the branch's
/// first complete ProductionLines census, so the totals cover its whole life;
/// otherwise they count from `since_hour`. `ended` says the line has left
/// ProductionLines: its rows are published once and then dropped (again at
/// the next publication if no written frame held them). Every
/// live line has at least one row, zero until it produces. `exact` is false
/// when a tick of the row could not be counted exactly (an unexpected caller,
/// variant or amount, or a delivery that does not match the credit), and
/// `units` then leaves that tick out; a conversion clamped to zero, which
/// delivers and credits zero, is exact when the delivery and credit otherwise
/// pair. Read in the lines' daily slot; a branch starts new totals, and the
/// table is unavailable for the rest of the branch once a tick has gone
/// unobserved, never partial.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ProductionLineOutputRow {
    pub line: u64,
    pub ordinal: u32,
    pub country: u32,
    pub variant: u64,
    pub mio: u64,
    pub units: i64,
    /// Retired: only the save comparison's self-checks read it.
    pub mio_credit: RetiredI64,
    pub since_hour: i32,
    pub observed_since_creation: bool,
    pub ended: bool,
    pub exact: bool,
    /// Exact signed fixed5 CREDIT input, excluding instant progress. Historical
    /// absence is NotRecorded; None is a current classification/total refusal.
    /// Delivery/MIO exactness is independent of these spending operands.
    pub spent_raw: Recorded<Option<i64>>,
    /// Retired: only the save comparison's self-checks read it.
    pub instant_raw: Recorded<RetiredOptionalI64>,
    /// [`OutputPurpose::raw`] of every tick in the row. Historical absence
    /// is NotRecorded; read it through [`Self::purpose`].
    pub purpose: Recorded<u8>,
}
typed_codec!(ProductionLineOutputRow, 96, |row| (row.line as u128) << 32 | row.ordinal as u128, |key| Self { line: (key >> 32) as u64, ordinal: key as u32, ..Default::default() },
{ 0 => (country): U32, 1 => (variant): U64, 2 => (mio): U64, 3 => (units): I64, 4 => (mio_credit): I64, 5 => (since_hour): I32, 6 => (observed_since_creation): Bool, 7 => (ended): Bool, 8 => (exact): Bool, 9 => (spent_raw): OptionalI64, 10 => (instant_raw): OptionalI64, 11 => (purpose): U8 },
|row| match row.purpose {
    Recorded::Value(raw) if OutputPurpose::from_raw(raw).is_none() => Err(invalid("production output purpose invalid")),
    _ => Ok(()),
});

impl ProductionLineOutputRow {
    /// The row's purpose; Unknown when it was not recorded.
    pub fn purpose(&self) -> OutputPurpose {
        match self.purpose {
            Recorded::Value(raw) => OutputPurpose::from_raw(raw).unwrap_or_default(),
            Recorded::NotRecorded => OutputPurpose::Unknown,
        }
    }
}

/// CombatLogEquipment: key country:group:kind:month:variant (120 bits), 2
/// payload mask bits. Sliced by country.
///
/// One variant's amount in one month entry of the country's Combat Log
/// for one orders group,
/// as its Equipment tab shows it. `country` is the log's raw country link,
/// `group` the orders group's reference id and `variant` the variant's; both
/// ids are the native reference ids (types 53 and 70). `kind` 0 to 3 is the
/// stored loss reason (0 lost in combat; 1, 2 and 3 the attrition the game
/// labels `COMBAT_LOG_EQ_LOST_ATTR_OTHER`, `_COMBAT` and `_TRAINING`); 4 is
/// equipment recovered after combat. `month` is the calendar month the game
/// merges an entry by (`year * 12` plus the zero-based month of its 365-day
/// calendar), `date` the raw game hour of the entry's first write and
/// `amount_raw` the fixed5 amount so far.
///
/// Only an array's last month entry grows, so a country's read holds the last
/// two entries of each array when its last complete read was in this or the
/// previous calendar month (every one that can have changed since), and
/// every retained entry otherwise and at a full hour. A row leaving the
/// table is therefore not undone losses: the entry stopped changing earlier,
/// or the game deleted it (`NGame.COMBAT_LOG_MAX_MONTHS` months old) or
/// deleted its group's log after the group was gone. The rows of a log the
/// daily prune deletes are its final values, published once with a read of
/// its country, and again if that read may not have been published. The native
/// writer requires an orders group and open country/feature gates (see
/// CombatLogState and SimulationFeatures); capture reads every held log.
/// A retention setting below 1 can delete entries still changing, which
/// consumers refuse. Only land divisions in orders groups are logged here;
/// SunkShips and SunkConvoys record ships and convoys, and air stays unknown.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CombatLogEquipmentRow {
    pub country: u32,
    pub group: u32,
    pub kind: u8,
    pub month: u16,
    pub variant: u32,
    pub date: i32,
    pub amount_raw: i64,
}
typed_codec!(CombatLogEquipmentRow, 120, |row| (row.country as u128) << 88 | (row.group as u128) << 56 | (row.kind as u128) << 48 | (row.month as u128) << 32 | row.variant as u128, |key| Self { country: (key >> 88) as u32, group: (key >> 56) as u32, kind: (key >> 48) as u8, month: (key >> 32) as u16, variant: key as u32, ..Default::default() },
 { 0 => (date): I32, 1 => (amount_raw): I64 }, |_row| Ok(()));

/// CombatLogState: singleton, 2 payload mask bits.
///
/// `viewing_country_raw` records the writer's signed country-gate operand
/// (below 1 admits every country), read every hour. A positive operand
/// admits countries in the live human roster (Players). This explains why
/// the game may not keep a country's land losses; it never selects what is
/// captured, and admission alone does not prove coverage: the army also
/// needs an orders group. Loss recording also requires feature 3; its
/// runtime mask is recorded separately by SimulationFeatures.
/// `prune_observed` is 1 when every
/// Combat Log the game deleted since the previous sample had its final rows
/// kept (the prune detour was installed and enabled throughout and nothing
/// overflowed or failed); 0 means a deleted log's last changes may be missing.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CombatLogStateRow {
    pub viewing_country_raw: i32,
    pub prune_observed: u8,
}
typed_codec!(CombatLogStateRow, 0, |_row| 0, |_key| Self { viewing_country_raw: Default::default(), prune_observed: Default::default() },
 { 0 => (viewing_country_raw): I32, 1 => (prune_observed): U8 }, |_row| Ok(()));

/// LendLeaseEquipment: key giver:exchange:kind:variant (104 bits), 2 payload
/// mask bits. Daily.
///
/// One variant of one lend-lease exchange (lend-lease exchange, the giver's country
/// resources outgoing list): `kind` 0 is what it has delivered (`total_delivered`), 1
/// what was sunk on the way (`equipment_sunk`); both fixed5 and only ever growing while
/// the exchange lives. `giver` is the giver's raw country reference, `exchange` the
/// exchange's reference id, `receiver` the receiver's raw country reference. The game
/// deletes the exchange when the lend-lease ends, taking its totals with it, so its rows
/// leave the table then; changes after its last read are not recorded.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct LendLeaseEquipmentRow {
    pub giver: u32,
    pub exchange: u32,
    pub kind: u8,
    pub variant: u32,
    pub receiver: u32,
    pub amount_raw: i64,
}
typed_codec!(LendLeaseEquipmentRow, 104, |row| (row.giver as u128) << 72 | (row.exchange as u128) << 40 | (row.kind as u128) << 32 | row.variant as u128, |key| Self { giver: (key >> 72) as u32, exchange: (key >> 40) as u32, kind: (key >> 32) as u8, variant: key as u32, ..Default::default() },
 { 0 => (receiver): U32, 1 => (amount_raw): I64 }, |_row| Ok(()));

/// Stockpiles: key country:ordinal (64 bits), 2 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct StockpileRow {
    pub country: u32,
    pub ordinal: u32,
    pub variant_handle: u64,
    pub amount_raw: i64,
}
typed_codec!(StockpileRow, 64, |row| (row.country as u128) << 32 | row.ordinal as u128, |key| Self { country: (key >> 32) as u32, ordinal: key as u32, variant_handle: Default::default(), amount_raw: Default::default() },
 { 0 => (variant_handle): U64, 1 => (amount_raw): I64 }, |_row| Ok(()));

/// DivisionEquipment: key division:ordinal (96 bits), 4 payload mask bits.
/// The daily table holds every division; the hourly `NewDivisionEquipment`
/// holds, in the same rows, only divisions minted since that daily walk.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DivisionEquipmentRow {
    pub division: u64,
    pub template: u64,
    pub country: u32,
    pub ordinal: u32,
    pub variant_handle: u64,
    pub amount_raw: i64,
}
typed_codec!(DivisionEquipmentRow, 96, |row| (row.division as u128) << 32 | row.ordinal as u128, |key| Self { division: (key >> 32) as u64, ordinal: key as u32, template: Default::default(), country: Default::default(), variant_handle: Default::default(), amount_raw: Default::default() },
 { 0 => (template): U64, 1 => (country): U32, 2 => (variant_handle): U64, 3 => (amount_raw): I64 }, |_row| Ok(()));

/// Daily: a country's local sub-unit clone whose ordered requirements differ
/// from its global definition's. Every other clone is absent, so a complete
/// table without a row means the global requirements apply. Keyed by the raw
/// country reference and the definition token.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CountryLocalSubunitRow {
    pub country: u32,
    pub token: u32,
    pub requirements: Vec<LocalSubunitRequirement>,
}
typed_codec!(CountryLocalSubunitRow, 64, |row| (row.country as u128) << 32 | row.token as u128,
 |key| Self { country: (key >> 32) as u32, token: key as u32, ..Default::default() },
 { 0 => (requirements): Opaque }, |_row| Ok(()));

/// Daily: every country's primary (kind 0) and foreign (kind 1) equipment
/// rosters and stored best-variant candidates (kind 2), in stored order
/// with repeats: the land-model conventional and fallback candidates.
/// Key country:kind:ordinal.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AvailableEquipmentVariantRow {
    pub country: u32,
    pub roster_kind: u8,
    pub ordinal: u32,
    /// Full equipment variant reference, joining `VariantDefinitions.id`.
    pub variant: u64,
}
typed_codec!(AvailableEquipmentVariantRow, 66, |row| (row.country as u128) << 34 | (row.roster_kind as u128) << 32 | row.ordinal as u128,
 |key| Self { country: (key >> 34) as u32, roster_kind: ((key >> 32) & 3) as u8, ordinal: key as u32, ..Default::default() },
 { 0 => (variant): U64 }, |row| { if row.roster_kind > 2 { return Err(invalid("equipment roster kind must be primary, foreign or best")); } Ok(()) });

/// Daily: one entry of a country's persistent unit-bonus list, in stored
/// order with repeats. The unit-bonus effect, doctrine rewards and milestones,
/// and special-project rewards append entries; the game rebuilds the
/// country's local sub-unit clones from the list, applying the entries in
/// order. A country without rows has an empty list in a complete table.
/// Keyed by the raw country reference and the entry's ordinal.
///
/// `source_kind` is none (0), special project (1), effect (2), grand doctrine
/// (3), sub-doctrine (4), doctrine reward (5) or doctrine milestone (6).
/// `source_id` is a lexer token; `source_number` is a doctrine reward's
/// ordinal or a milestone's track index; `label` is the effect's name. Not
/// recorded: each target's requirements (CountryLocalSubunits holds the
/// clones' effective requirements), terrain and special adjusters, priorities,
/// sprite, transport restriction and own-fuel multiplier, none of which the
/// clone rebuild applies.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CountryUnitBonusRow {
    pub country: u32,
    pub ordinal: u32,
    pub source_kind: u8,
    pub source_id: u32,
    pub source_number: i32,
    pub label: Vec<u8>,
    /// Every exact sub-unit target, then every sub-unit category target, each
    /// in stored order with repeats.
    pub targets: Vec<UnitBonusTarget>,
}
typed_codec!(CountryUnitBonusRow, 64, |row| (row.country as u128) << 32 | row.ordinal as u128,
 |key| Self { country: (key >> 32) as u32, ordinal: key as u32, ..Default::default() },
 { 0 => (source_kind): U8, 1 => (source_id): U32, 2 => (source_number): I32, 3 => (label): Bytes, 4 => (targets): Opaque }, |_row| Ok(()));

/// VariantModules: key variant:source_ordinal (96 bits), 3 payload mask bits.
/// The module's name is content: `definition_token` spelled by `LexicalTokens`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct VariantModuleRow {
    pub variant: u64,
    pub source_ordinal: u32,
    pub slot: u32,
    pub definition_token: u32,
    pub definition_active: u8,
}
typed_codec!(VariantModuleRow, 96, |row| (row.variant as u128) << 32 | row.source_ordinal as u128, |key| Self { variant: (key >> 32) as u64, source_ordinal: key as u32, slot: Default::default(), definition_token: Default::default(), definition_active: Default::default() },
 { 0 => (slot): U32, 1 => (definition_token): U32, 2 => (definition_active): U8 }, |_row| Ok(()));

/// VariantStats: key variant (64 bits), 13 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct VariantStatsRow {
    pub variant: u64,
    pub values: [i64; 10],
    pub organisation: u64,
    pub organisation_trait_count: i32,
    pub organisation_bonuses: Vec<IndustrialOrganisationBonus>,
}
typed_codec!(VariantStatsRow, 64, |row| row.variant as u128, |key| Self { variant: key as u64, ..Default::default() },
 { 0 => (values[0]): I64, 1 => (values[1]): I64, 2 => (values[2]): I64, 3 => (values[3]): I64, 4 => (values[4]): I64, 5 => (values[5]): I64, 6 => (values[6]): I64, 7 => (values[7]): I64, 8 => (values[8]): I64, 9 => (values[9]): I64, 10 => (organisation): U64, 11 => (organisation_trait_count): I32, 12 => (organisation_bonuses): Opaque }, |_row| Ok(()));

/// VariantUpgrades: key variant:source_ordinal (96 bits), 2 payload mask bits.
/// The upgrade's name is content: `definition_token` spelled by `LexicalTokens`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct VariantUpgradeRow {
    pub variant: u64,
    pub source_ordinal: u32,
    pub definition_token: i32,
    pub level: u8,
}
typed_codec!(VariantUpgradeRow, 96, |row| (row.variant as u128) << 32 | row.source_ordinal as u128, |key| Self { variant: (key >> 32) as u64, source_ordinal: key as u32, definition_token: Default::default(), level: Default::default() },
 { 0 => (definition_token): I32, 1 => (level): U8 }, |_row| Ok(()));

/// ArmyUpgradeGroups: key army (64 bits), 2 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ArmyUpgradeGroupRow {
    pub country: u32,
    pub army: u64,
    pub priority: u32,
}
typed_codec!(ArmyUpgradeGroupRow, 64, |row| row.army as u128, |key| Self { army: key as u64, country: Default::default(), priority: Default::default() },
 { 0 => (country): U32, 1 => (priority): U32 }, |_row| Ok(()));

/// ArmyUpgradeRequests: key army:ordinal (96 bits), 4 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ArmyUpgradeRequestRow {
    pub country: u32,
    pub army: u64,
    pub ordinal: u32,
    pub equipment: u64,
    pub request_raw: i64,
    pub total_raw: i64,
}
typed_codec!(ArmyUpgradeRequestRow, 96, |row| (row.army as u128) << 32 | row.ordinal as u128, |key| Self { army: (key >> 32) as u64, ordinal: key as u32, country: Default::default(), equipment: Default::default(), request_raw: Default::default(), total_raw: Default::default() },
 { 0 => (country): U32, 1 => (equipment): U64, 2 => (request_raw): I64, 3 => (total_raw): I64 }, |_row| Ok(()));

/// ArmyUpgradeDeliveries: key army:ordinal (96 bits), 5 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ArmyUpgradeDeliveryRow {
    pub country: u32,
    pub army: u64,
    pub ordinal: u32,
    pub status: u32,
    pub progress_raw: Option<i64>,
    pub total_progress_raw: i64,
    pub allow_zero_entries: u8,
}
typed_codec!(ArmyUpgradeDeliveryRow, 96, |row| (row.army as u128) << 32 | row.ordinal as u128, |key| Self { army: (key >> 32) as u64, ordinal: key as u32, country: Default::default(), status: Default::default(), progress_raw: Default::default(), total_progress_raw: Default::default(), allow_zero_entries: Default::default() },
 { 0 => (country): U32, 1 => (status): U32, 2 => (progress_raw): OptionalI64, 3 => (total_progress_raw): I64, 4 => (allow_zero_entries): U8 }, |_row| Ok(()));

/// ArmyUpgradeProduced: key army:delivery_ordinal:ordinal (128 bits), 3 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ArmyUpgradeProducedRow {
    pub country: u32,
    pub army: u64,
    pub delivery_ordinal: u32,
    pub ordinal: u32,
    pub equipment: u64,
    pub amount_raw: i64,
}
typed_codec!(ArmyUpgradeProducedRow, 128, |row| (row.army as u128) << 64 | (row.delivery_ordinal as u128) << 32 | row.ordinal as u128, |key| Self { army: (key >> 64) as u64, delivery_ordinal: (key >> 32) as u32, ordinal: key as u32, country: Default::default(), equipment: Default::default(), amount_raw: Default::default() },
 { 0 => (country): U32, 1 => (equipment): U64, 2 => (amount_raw): I64 }, |_row| Ok(()));

/// CountryFuelConsumers: key country:index (64 bits), 3 payload mask bits.
///
/// One entry of a country's fuel consumer array, in array order; `index` is
/// the position the save writes. Requested and received fuel are fixed5.
/// `CountryFuel` comes from the same read of each fuel status.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CountryFuelConsumerRow {
    pub country: u32,
    pub index: u32,
    pub priority: i32,
    pub requested_raw: i64,
    pub received_raw: i64,
}
typed_codec!(CountryFuelConsumerRow, 64, |row| (row.country as u128) << 32 | row.index as u128, |key| Self { country: (key >> 32) as u32, index: key as u32, ..Default::default() },
 { 0 => (priority): I32, 1 => (requested_raw): I64, 2 => (received_raw): I64 }, |_row| Ok(()));

/// CountryFactories: key country (32 bits), 40 payload mask bits.
///
/// One country's stored factory totals, as the game last recalculated them:
/// its military, naval and civilian factory groups, the civilian licence and
/// trade exports, and what each civilian consumer took (consumer goods,
/// national projects, licence imports, trade imports and market contract
/// payments). These are the numbers the top bar, the factory tooltips and the
/// factory triggers read, not a derivation from state buildings. `_raw` fields
/// are fixed5 (x100000); the game shows each truncated toward zero. `levels` is
/// the sum of building levels counted, before any output factor.
/// One row per canonical country, keyed by raw reference: consumers join
/// through CountryLinks, and a missing row is unknown. Read hourly.
/// Buildings count through their content flags (`military_production`,
/// `naval_production`, `general_production`) in controlled states and offsite
/// buildings; state building levels are weighted by the
/// state's output factor for the fixed5 totals. Each group keeps total,
/// controlled, owned and damaged factories, factories assigned to lines or
/// constructions, withheld factories, and subject and other-country transfers.
///
/// With `trunc(x)` the game's `(int)(x / 100000)` and T, D, C and O the
/// truncated total, damaged, controlled and owned, the construction tab
/// shows civilian Total T, Owned O, From occupation C - O,
/// Damaged D, Unused max(0, T - used - unavailable - D) and In use
/// D - (used + unavailable), and lists each withheld line (consumer goods,
/// contracts, projects, licence and trade imports, overlord and other
/// countries, construction). Its header shows
/// (T - D - max(0, T - D - unavailable - used)) / (T - D).
/// The top-bar tooltip shows, per group,
/// T - D - to_overlord - to_others as total and
/// min(that, unavailable + used - to_overlord - to_others) as used, with From
/// trade the licence plus trade exports. The top-bar number is
/// T(military) + T(civilian) + T(naval) - civilian trade and licence
/// exports - the civilian and military to_overlord and to_others.
/// `unavailable` already includes every consumer and both transfers out;
/// consumer goods is the game's allocation, not a percentage to round.
/// `num_of_controlled_factories` and scripted-text seeds sum the three
/// stored group-head levels with i32 wrapping, not fixed5 totals.
/// No group field has a text-save counterpart; the oracle compares levels
/// only with saved building inputs.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CountryFactoriesRow {
    pub country: u32,
    pub military_levels: i32,
    pub military_total_raw: i64,
    pub military_controlled_raw: i64,
    pub military_owned_raw: i64,
    pub military_damaged_raw: i64,
    pub military_used: i32,
    pub military_unavailable: i32,
    pub military_from_subjects: i32,
    pub military_to_overlord: i32,
    pub military_from_others: i32,
    pub military_to_others: i32,
    pub naval_levels: i32,
    pub naval_total_raw: i64,
    pub naval_controlled_raw: i64,
    pub naval_owned_raw: i64,
    pub naval_damaged_raw: i64,
    pub naval_used: i32,
    pub naval_unavailable: i32,
    pub naval_from_subjects: i32,
    pub naval_to_overlord: i32,
    pub naval_from_others: i32,
    pub naval_to_others: i32,
    pub civilian_levels: i32,
    pub civilian_total_raw: i64,
    pub civilian_controlled_raw: i64,
    pub civilian_owned_raw: i64,
    pub civilian_damaged_raw: i64,
    pub civilian_used: i32,
    pub civilian_unavailable: i32,
    pub civilian_from_subjects: i32,
    pub civilian_to_overlord: i32,
    pub civilian_from_others: i32,
    pub civilian_to_others: i32,
    pub civilian_license_exports: i32,
    pub civilian_trade_exports: i32,
    pub consumer_goods: i32,
    pub special_projects: i32,
    pub license_imports: i32,
    pub trade_imports: i32,
    pub contract_payments: i32,
}
typed_codec!(CountryFactoriesRow, 32, |row| row.country as u128, |key| Self { country: key as u32, ..Default::default() },
 { 0 => (military_levels): I32, 1 => (military_total_raw): I64, 2 => (military_controlled_raw): I64, 3 => (military_owned_raw): I64, 4 => (military_damaged_raw): I64, 5 => (military_used): I32, 6 => (military_unavailable): I32, 7 => (military_from_subjects): I32, 8 => (military_to_overlord): I32, 9 => (military_from_others): I32, 10 => (military_to_others): I32,
   11 => (naval_levels): I32, 12 => (naval_total_raw): I64, 13 => (naval_controlled_raw): I64, 14 => (naval_owned_raw): I64, 15 => (naval_damaged_raw): I64, 16 => (naval_used): I32, 17 => (naval_unavailable): I32, 18 => (naval_from_subjects): I32, 19 => (naval_to_overlord): I32, 20 => (naval_from_others): I32, 21 => (naval_to_others): I32,
   22 => (civilian_levels): I32, 23 => (civilian_total_raw): I64, 24 => (civilian_controlled_raw): I64, 25 => (civilian_owned_raw): I64, 26 => (civilian_damaged_raw): I64, 27 => (civilian_used): I32, 28 => (civilian_unavailable): I32, 29 => (civilian_from_subjects): I32, 30 => (civilian_to_overlord): I32, 31 => (civilian_from_others): I32, 32 => (civilian_to_others): I32,
   33 => (civilian_license_exports): I32, 34 => (civilian_trade_exports): I32, 35 => (consumer_goods): I32, 36 => (special_projects): I32, 37 => (license_imports): I32, 38 => (trade_imports): I32, 39 => (contract_payments): I32 }, |_row| Ok(()));

/// ResourceDefinitions: key id (32 bits), 4 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ResourceDefinitionRow {
    pub id: u32,
    pub token: u32,
    pub active: u8,
    pub factory_cost: i64,
}
typed_codec!(ResourceDefinitionRow, 32, |row| row.id as u128, |key| Self { id: key as u32, token: Default::default(), active: Default::default(), factory_cost: Default::default() },
 { 0 => (token): U32, 1 => (active): U8, 2 => (factory_cost): I64 }, |_row| Ok(()));

/// CountryResources: key country:resource (64 bits), 9 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CountryResourceRow {
    pub country: u32,
    pub resource: u32,
    pub produced: i64,
    pub adjustment: i64,
    pub imported: i64,
    pub used: i64,
    pub special_total: i64,
    pub production_demand: i64,
    pub to_export: i64,
    pub base_export: i64,
    pub exported: i64,
}
typed_codec!(CountryResourceRow, 64, |row| (row.country as u128) << 32 | row.resource as u128, |key| Self { country: (key >> 32) as u32, resource: key as u32, produced: Default::default(), adjustment: Default::default(), imported: Default::default(), used: Default::default(), special_total: Default::default(), production_demand: Default::default(), to_export: Default::default(), base_export: Default::default(), exported: Default::default() },
 { 0 => (produced): I64, 1 => (adjustment): I64, 2 => (imported): I64, 3 => (used): I64, 4 => (special_total): I64, 5 => (production_demand): I64, 6 => (to_export): I64, 7 => (base_export): I64, 8 => (exported): I64 }, |_row| Ok(()));

/// ResourceExchanges: key id (64 bits), 14 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ResourceExchangeRow {
    pub id: u64,
    pub exporter: u32,
    pub importer: u32,
    pub resource: u32,
    pub import_ordinal: u32,
    pub export_ordinal: u32,
    pub convoys_assigned: i32,
    pub convoys_required: i32,
    pub efficiency_a: i64,
    pub efficiency_b: i64,
    pub delivered: i64,
    pub required_factories: i32,
    pub assigned_factories: i32,
    pub no_trade_influence: u8,
    pub no_resources: u8,
}
typed_codec!(ResourceExchangeRow, 64, |row| row.id as u128, |key| Self { id: key as u64, exporter: Default::default(), importer: Default::default(), resource: Default::default(), import_ordinal: Default::default(), export_ordinal: Default::default(), convoys_assigned: Default::default(), convoys_required: Default::default(), efficiency_a: Default::default(), efficiency_b: Default::default(), delivered: Default::default(), required_factories: Default::default(), assigned_factories: Default::default(), no_trade_influence: Default::default(), no_resources: Default::default() },
 { 0 => (exporter): U32, 1 => (importer): U32, 2 => (resource): U32, 3 => (import_ordinal): U32, 4 => (export_ordinal): U32, 5 => (convoys_assigned): I32, 6 => (convoys_required): I32, 7 => (efficiency_a): I64, 8 => (efficiency_b): I64, 9 => (delivered): I64, 10 => (required_factories): I32, 11 => (assigned_factories): I32, 12 => (no_trade_influence): U8, 13 => (no_resources): U8 }, |_row| Ok(()));

/// PurchaseContracts: key id (64 bits), 11 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct PurchaseContractRow {
    pub id: u64,
    pub seller: u32,
    pub buyer: u32,
    pub speed: i32,
    pub convoys_assigned: i32,
    pub convoys_total: i32,
    pub route_efficiency_sum_raw: i64,
    pub route_days: i32,
    pub factory_cic_progress_raw: i64,
    pub equipment_transfer_progress_raw: i64,
    pub collected_raw: i64,
    pub completed_deliveries: i32,
}
typed_codec!(PurchaseContractRow, 64, |row| row.id as u128, |key| Self { id: key as u64, seller: Default::default(), buyer: Default::default(), speed: Default::default(), convoys_assigned: Default::default(), convoys_total: Default::default(), route_efficiency_sum_raw: Default::default(), route_days: Default::default(), factory_cic_progress_raw: Default::default(), equipment_transfer_progress_raw: Default::default(), collected_raw: Default::default(), completed_deliveries: Default::default() },
 { 0 => (seller): U32, 1 => (buyer): U32, 2 => (speed): I32, 3 => (convoys_assigned): I32, 4 => (convoys_total): I32, 5 => (route_efficiency_sum_raw): I64, 6 => (route_days): I32, 7 => (factory_cic_progress_raw): I64, 8 => (equipment_transfer_progress_raw): I64, 9 => (collected_raw): I64, 10 => (completed_deliveries): I32 }, |_row| Ok(()));

/// StateBuildings: key location:building_type (64 bits), 7 payload mask bits.
/// ProvinceBuildings: key location:building_type (64 bits), 7 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct BuildingRow {
    pub location: u32,
    pub building_type: u32,
    pub ordinal: u32,
    pub status_flags: u32,
    pub country: u32,
    pub level: i16,
    pub healthy_level: i16,
    pub partial_health: i64,
    /// Listed in the location's active building vector, which building scans walk.
    pub active: bool,
}
typed_codec!(BuildingRow, 64, |row| (row.location as u128) << 32 | row.building_type as u128, |key| Self { location: (key >> 32) as u32, building_type: key as u32, ordinal: Default::default(), status_flags: Default::default(), country: Default::default(), level: Default::default(), healthy_level: Default::default(), partial_health: Default::default(), active: Default::default() },
 { 0 => (ordinal): U32, 1 => (status_flags): U32, 2 => (country): U32, 3 => (level): I16, 4 => (healthy_level): I16, 5 => (partial_health): I64, 6 => (active): Bool }, |_row| Ok(()));

/// StateBaseResources: key state:resource (64 bits), 2 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct StateBaseResourceRow {
    pub state: u32,
    pub resource: u32,
    pub ordinal: u32,
    pub amount: i64,
}
typed_codec!(StateBaseResourceRow, 64, |row| (row.state as u128) << 32 | row.resource as u128, |key| Self { state: (key >> 32) as u32, resource: key as u32, ordinal: Default::default(), amount: Default::default() },
 { 0 => (ordinal): U32, 1 => (amount): I64 }, |_row| Ok(()));

/// BuildingConstruction: key id (64 bits), 15 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct BuildingConstructionRow {
    pub id: u64,
    pub country: u32,
    pub queue_ordinal: u32,
    pub active_factories: i32,
    pub cost: i64,
    pub speed: i64,
    pub produced: i64,
    pub priority: i32,
    pub amount: i32,
    pub created_hour: i32,
    pub building_type: u32,
    pub target_flags: u32,
    pub target_location: u32,
    pub target_state: u32,
    pub target_country: u32,
    pub repair_levels: i32,
}
typed_codec!(BuildingConstructionRow, 64, |row| row.id as u128, |key| Self { id: key as u64, country: Default::default(), queue_ordinal: Default::default(), active_factories: Default::default(), cost: Default::default(), speed: Default::default(), produced: Default::default(), priority: Default::default(), amount: Default::default(), created_hour: Default::default(), building_type: Default::default(), target_flags: Default::default(), target_location: Default::default(), target_state: Default::default(), target_country: Default::default(), repair_levels: Default::default() },
 { 0 => (country): U32, 1 => (queue_ordinal): U32, 2 => (active_factories): I32, 3 => (cost): I64, 4 => (speed): I64, 5 => (produced): I64, 6 => (priority): I32, 7 => (amount): I32, 8 => (created_hour): I32, 9 => (building_type): U32, 10 => (target_flags): U32, 11 => (target_location): U32, 12 => (target_state): U32, 13 => (target_country): U32, 14 => (repair_levels): I32 }, |_row| Ok(()));

/// RailwayConstruction: key id (64 bits), 15 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct RailwayConstructionRow {
    pub id: u64,
    pub country: u32,
    pub queue_ordinal: u32,
    pub active_factories: i32,
    pub cost: i64,
    pub speed: i64,
    pub produced: i64,
    pub priority: i32,
    pub amount: i32,
    pub created_hour: i32,
    pub base_level: i32,
    pub target_level: i32,
    pub cancelled: u8,
    pub current_segment: i32,
    pub remaining_cost: i64,
    pub route: Vec<u32>,
}
typed_codec!(RailwayConstructionRow, 64, |row| row.id as u128, |key| Self { id: key as u64, country: Default::default(), queue_ordinal: Default::default(), active_factories: Default::default(), cost: Default::default(), speed: Default::default(), produced: Default::default(), priority: Default::default(), amount: Default::default(), created_hour: Default::default(), base_level: Default::default(), target_level: Default::default(), cancelled: Default::default(), current_segment: Default::default(), remaining_cost: Default::default(), route: Default::default() },
 { 0 => (country): U32, 1 => (queue_ordinal): U32, 2 => (active_factories): I32, 3 => (cost): I64, 4 => (speed): I64, 5 => (produced): I64, 6 => (priority): I32, 7 => (amount): I32, 8 => (created_hour): I32, 9 => (base_level): I32, 10 => (target_level): I32, 11 => (cancelled): U8, 12 => (current_segment): I32, 13 => (remaining_cost): I64, 14 => (route): U32List }, |_row| Ok(()));

/// StateResourceInputs: key state (32 bits).
///
/// The resource-count operands shared across a state's resources. The game
/// computes the amount on demand; the viewer computes it from these rows,
/// `StateResources`, `StateBaseResources`, `StateBuildings`,
/// `CountryBuildingResources` and content.
///
/// Sliced (family `raid_state`, owner: the state's native ID, with `StateResources`,
/// `StateDynamicModifiers` and `StateCores`): each hour reads a daily 1/24 slice of
/// states, an owner new to the census, the owners of a read that was not published and
/// every state a hooked native writer touched since the previous hour (the collector's
/// `raid_changes`); an hour whose marks are not certified reads every state. So every
/// value equals the game's at every hour (owner decision 2026-10-11: the game's offer
/// cache re-checks every offer at least hourly), except `supply_route`, which the supply
/// update rewrites every hour and no writer names: it is the value at the state's read
/// hour, up to a day old. Every read state has a row here, so this table is the family's
/// hour source: its owners' read hours are the ones in `SliceReads`
/// (`replay_data::sliced::read_hour`); an unread owner's rows are current.
///
/// A controlled state that bears no resources has neither
/// `infrastructure_level` nor `supply_route` (nor `origin`) read, and no icon can depend on any of them. For each resource
/// Resource accumulation scales the base amount
/// `(template + pool + additive under the gate) x factor x global factor` by
/// the infrastructure factor and then the supply-route factor, clamps at zero,
/// subtracts the cost and adds the building grants. Without resources the
/// template has no amount, every pool value is zero and the gate drops the
/// additive keys, so every base, and every scaled product, is zero whatever
/// the level or the route. The viewer reads `infrastructure_level` and
/// `supply_route` only as `OriginFactors` in the viewer's
/// `state_resources::state_resource_amount` (through
/// `StateResources::amount`, the amount a `<resource> <op> N` trigger
/// compares in the raid script and the tactic triggers), where they
/// multiply that zero product. An uncontrolled state
/// has no origin, so neither factor applies to it (`origin` is zero).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct StateResourceInputRow {
    pub local_manpower: Recorded<Option<i64>>,
    pub local_noncore_manpower: Recorded<Option<i64>>,
    pub recruitment_add: Recorded<Option<i64>>,
    pub recruitment_factor: Recorded<Option<i64>>,
    pub state: u32,
    /// The additive keys count only when this state gate is set.
    pub gate: u8,
    /// Effective state modifier 0x223, fixed5.
    pub global_factor: i64,
    /// The controller has a `resource origin` for the state; without one the
    /// infrastructure and supply factors are one.
    pub origin: u8,
    /// The infrastructure building's level, damage included; read only with an origin, absent when
    /// the state has no such building.
    pub infrastructure_level: Option<i32>,
    /// Effective modifier 0x292 for the controller, fixed5; read only with an
    /// infrastructure level, zero otherwise.
    pub infrastructure_modifier: i64,
    /// Whether the state's maximum province node value is positive; read only with an origin.
    pub supply_route: u8,
}
typed_codec!(StateResourceInputRow, 32, |row| row.state as u128,
 |key| Self { state: key as u32, ..Default::default() },
 { 0 => (gate): U8, 1 => (global_factor): I64, 2 => (origin): U8, 3 => (infrastructure_level): OptionalI32, 4 => (infrastructure_modifier): I64, 5 => (supply_route): U8, 6 => (local_manpower): OptionalI64, 7 => (local_noncore_manpower): OptionalI64, 8 => (recruitment_add): OptionalI64, 9 => (recruitment_factor): OptionalI64 }, |_row| Ok(()));

/// StateResources: key state:resource lexer token (64 bits).
///
/// A resource's stored operands in one state; a resource whose four values
/// are all zero has no row. `pool` is the save's state `resources`; the
/// rest are effective state modifiers keyed by the resource definition's
/// additive, factor and global-factor keys. All fixed5.
///
/// Sliced, with `StateResourceInputs`: every value equals the game's at
/// every hour.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct StateResourceRow {
    pub state: u32,
    pub resource: u32,
    pub pool: i64,
    pub additive: i64,
    pub factor: i64,
    pub cost: i64,
}
typed_codec!(StateResourceRow, 64, |row| (row.state as u128) << 32 | row.resource as u128,
 |key| Self { state: (key >> 32) as u32, resource: key as u32, ..Default::default() },
 { 0 => (pool): I64, 1 => (additive): I64, 2 => (factor): I64, 3 => (cost): I64 }, |_row| Ok(()));

/// CountryBuildingResources: key country:ordinal (64 bits).
///
/// A country's building-resource overrides, one row per element; an entry
/// without elements has one row with `count` and `resource` zero. Read for
/// state controllers only. For a building whose first entry has elements,
/// the first matching element's amount is added to each grant, keeping
/// positive sums only.
///
/// Sliced (family `raid_country_buildings`, owner: the country's native
/// ID): each hour reads a daily 1/24 slice of countries, an owner new to
/// the census, the owners of a read that was not published, every country
/// whose overrides a hooked writer touched since the previous hour and the
/// controller of every state the `raid_state` family reads, so the rows
/// equal the game's at every hour. Only controllers are walked;
/// `SliceReads` holds the read hour of each country read, rows or none.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CountryBuildingResourceRow {
    pub country: u32,
    pub ordinal: u32,
    pub entry: u32,
    /// The building type's lexer token.
    pub building: u32,
    pub count: u32,
    /// The resource's lexer token.
    pub resource: u32,
    pub amount: i64,
}
typed_codec!(CountryBuildingResourceRow, 64, |row| (row.country as u128) << 32 | row.ordinal as u128,
 |key| Self { country: (key >> 32) as u32, ordinal: key as u32, ..Default::default() },
 { 0 => (entry): U32, 1 => (building): U32, 2 => (count): U32, 3 => (resource): U32, 4 => (amount): I64 }, |_row| Ok(()));

/// ResourceOrigins: key state:country (64 bits).
///
/// A resource origin the resources map icon reads: the controller's own origin of the state
/// (`list` 0) or a rights holder's (`list` 1); `country` is the country whose list holds it.
/// `efficiency` is the origin's stored efficiency product the icon multiplies by,
/// `route_factor` the supply-route factor fixed when the game loaded or the origin was built,
/// and `local_resources_factor` the effective `local_resources_factor` for the state and
/// `country`. All fixed5. `bad` is whether the origin's line draws in the bad-route colour:
/// its convoy efficiency product below
/// `TRADE_ROUTE_BAD_EFFICIENCY_THRESHOLD`; `convoys_assigned` and `convoys_required` are its
/// convoy counts. Its route is TradeRoutes' class 0 route of (`country`, `state`).
///
/// Hourly: the game refreshes origins in its hourly trade task.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ResourceOriginRow {
    /// Zero-based position in this country's own or rights-holder origin list.
    pub ordinal: u32,
    pub state: u32,
    pub country: u32,
    pub list: u8,
    pub efficiency: i64,
    pub route_factor: i64,
    pub local_resources_factor: i64,
    pub bad: bool,
    pub convoys_assigned: i32,
    pub convoys_required: i32,
}
typed_codec!(ResourceOriginRow, 64, |row| (row.state as u128) << 32 | row.country as u128,
 |key| Self { state: (key >> 32) as u32, country: key as u32, ..Default::default() },
 { 0 => (list): U8, 1 => (efficiency): I64, 2 => (route_factor): I64, 3 => (local_resources_factor): I64, 4 => (bad): Bool, 5 => (convoys_assigned): I32, 6 => (convoys_required): I32, 7 => (ordinal): U32 },
 |row| if row.list > 1 { Err(invalid("resource origin list is neither own (0) nor rights (1)")) } else { Ok(()) });

/// ResourceRights: key state:resource (64 bits).
///
/// A resource right on a state: the state controller's origin gives `resource` (the
/// one-based resource database index) to `holder`. Hourly.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ResourceRightRow {
    /// Zero-based position in the origin's resource-rights list.
    pub ordinal: u32,
    pub state: u32,
    pub resource: u32,
    pub holder: u32,
}
typed_codec!(ResourceRightRow, 64, |row| (row.state as u128) << 32 | row.resource as u128,
 |key| Self { state: (key >> 32) as u32, resource: key as u32, ..Default::default() },
 { 0 => (holder): U32, 1 => (ordinal): U32 }, |_row| Ok(()));

/// CountryOilFactors: key country (32 bits).
///
/// The country's effective `production_oil_factor`, fixed5, which scales the oil resource's
/// building grants in the states it controls; a zero factor has no row. Hourly.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CountryOilFactorRow {
    pub country: u32,
    pub factor: i64,
}
typed_codec!(CountryOilFactorRow, 32, |row| row.country as u128,
 |key| Self { country: key as u32, ..Default::default() },
 { 0 => (factor): I64 }, |_row| Ok(()));

/// StateTemporaryResources: key state:resource lexer token (64 bits).
///
/// A state's temporary resources, the resource definition's temporary key in the state's
/// modifiers, read only while the state's temporary-resource list is not empty or its flag
/// is set, which is when the resources map icon adds them; a zero amount has no row.
/// Fixed5. Hourly.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct StateTemporaryResourceRow {
    pub state: u32,
    pub resource: u32,
    pub amount: i64,
}
typed_codec!(StateTemporaryResourceRow, 64, |row| (row.state as u128) << 32 | row.resource as u128,
 |key| Self { state: (key >> 32) as u32, resource: key as u32, ..Default::default() },
 { 0 => (amount): I64 }, |_row| Ok(()));

/// ConvoyClients: key client handle id (32 bits).
///
/// Every convoy client a trade-route line draws for, other than resource origins:
/// `category` 1 a naval supply transfer of `owner`, 2 an import of `owner` from `other`,
/// 4 lend-lease from `owner` to `other`, 6 a market contract sold by `owner` to `other`.
/// Its route is TradeRoutes' (`route_class`, `route_country`, `route_other`). `bad` is
/// whether its line draws in the bad-route colour: its efficiency product below
/// `TRADE_ROUTE_BAD_EFFICIENCY_THRESHOLD`; `convoys_assigned` and `convoys_required` are its
/// convoy counts. Hourly.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ConvoyClientRow {
    /// Independent positions in the owner's and counterparty's directional lists.
    /// Imports/exports instead use ResourceExchanges' existing ordinals; None refuses
    /// any other layer that needs this position, rather than using handle order.
    pub owner_ordinal: Option<u32>,
    pub other_ordinal: Option<u32>,
    pub handle: u32,
    pub category: u8,
    pub owner: u32,
    pub other: u32,
    pub route_class: u8,
    pub route_country: u32,
    pub route_other: u32,
    pub bad: bool,
    pub convoys_assigned: i32,
    pub convoys_required: i32,
}
typed_codec!(ConvoyClientRow, 32, |row| row.handle as u128,
 |key| Self { handle: key as u32, ..Default::default() },
 { 0 => (category): U8, 1 => (owner): U32, 2 => (other): U32, 3 => (route_class): U8, 4 => (route_country): U32, 5 => (route_other): U32, 6 => (bad): Bool, 7 => (convoys_assigned): I32, 8 => (convoys_required): I32, 9 => (owner_ordinal): OptionalU32, 10 => (other_ordinal): OptionalU32 }, |_row| Ok(()));

/// RegionalConvoyEfficiency: key country:region (64 bits).
///
/// A country's delivery efficiency in one strategic sea region, fixed5, as its strategic
/// navy's regional convoy record holds it (full for a region beyond its records). A bad
/// trade-route line draws a region in the hotspot colour for a viewing country whose
/// efficiency there is below `TRADE_ROUTE_REGIONAL_BAD_EFFICIENCY_THRESHOLD`. Read for every
/// country taking part in a bad naval route, in each sea region of its path. Hourly.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct RegionalConvoyEfficiencyRow {
    pub country: u32,
    pub region: u32,
    pub efficiency: i64,
}
typed_codec!(RegionalConvoyEfficiencyRow, 64, |row| (row.country as u128) << 32 | row.region as u128,
 |key| Self { country: (key >> 32) as u32, region: key as u32, ..Default::default() },
 { 0 => (efficiency): I64 }, |_row| Ok(()));

/// TradeRoutes: key country:class:other (72 bits).
///
/// Every route a convoy client draws: class 0 a resource origin's (`country` the origin's
/// list owner, `other` its state), 1 a delivery route (`country` the exporter, `other` the
/// importer), 2 a naval supply transfer's (`country`, `other` the client's handle id). A
/// naval route (`kind` 2) draws from port province `from` through the strategic sea regions
/// in `path` to port province `to`; any other kind from state `from` through the states in
/// `path` to state `to`. A route that draws nothing has `from`, `to` and `path` empty.
///
/// Sliced (family `trade_routes`, owner: `country`): each hour reads the daily slice, an
/// owner new to the census or whose set of referenced routes changed, and every owner one
/// of whose routes a hooked route writer touched since the previous hour; an hour whose take
/// of those marks does not certify the previous hour's read reads every owner.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct TradeRouteRow {
    pub country: u32,
    pub class: u8,
    pub other: u32,
    pub kind: u8,
    pub from: u32,
    pub to: u32,
    pub path: Vec<u32>,
    /// The native source/destination states used by market route grouping, independent
    /// of naval port provinces. None on routes without a market client or a null state.
    pub source_state: Option<u32>,
    pub destination_state: Option<u32>,
}
typed_codec!(TradeRouteRow, 72, |row| (row.country as u128) << 40 | (row.class as u128) << 32 | row.other as u128,
 |key| Self { country: (key >> 40) as u32, class: (key >> 32) as u8, other: key as u32, ..Default::default() },
 { 0 => (kind): U8, 1 => (from): U32, 2 => (to): U32, 3 => (path): U32List, 4 => (source_state): OptionalU32, 5 => (destination_state): OptionalU32 }, |_row| Ok(()));
/// StockpileArchetypes: key canonical slot:archetype token. The stock of each archetype a raid type
/// lists as essential equipment, in a country's stockpile pool (the production status's first
/// stockpile array), which the raid equipment-requirement check compares with each amount:
/// archetype quantity in whole units, capped at the largest amount any raid type requires of the
/// archetype, so the raid offer's verdict and printed shortfall stay exact and the value changes
/// only while the country is short. An essential archetype without a row has no stock. Hourly,
/// every country.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct StockpileArchetypeRow {
    pub canonical_slot: u32,
    pub archetype: u32,
    pub units: i32,
}
typed_codec!(StockpileArchetypeRow, 64, |row| (row.canonical_slot as u128) << 32 | row.archetype as u128, |key| Self { canonical_slot: (key >> 32) as u32, archetype: key as u32, ..Default::default() },
 { 0 => (units): I32 }, |_row| Ok(()));

/// ProductionLicenses: key country:ordinal (64 bits), 1 payload mask bit.
/// The licensor and origin are the licensed variant's own creator/origin links,
/// recorded once in VariantDefinitions and joined by this handle.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ProductionLicenseRow {
    pub country: u32,
    pub ordinal: u32,
    pub variant: u64,
}
typed_codec!(ProductionLicenseRow, 64, |row| (row.country as u128) << 32 | row.ordinal as u128, |key| Self { country: (key >> 32) as u32, ordinal: key as u32, variant: Default::default() },
 { 0 => (variant): U64 }, |_row| Ok(()));

/// ProductionLicenseRecords: key owner:ordinal (64 bits), 7 payload mask bits.
/// One granted licence relation: `owner` is the licensee whose roster stores it
/// and `giver` the licensor. The engine also keeps the same object in the
/// giver's reverse array, which is never serialized and is not recorded.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ProductionLicenseRecordRow {
    pub owner: u32,
    pub ordinal: u32,
    pub giver: u32,
    pub lended_cic: i32,
    pub required_cic: i32,
    /// Native engine date hour, not a calendar conversion.
    pub start_date: i32,
    pub parent: u64,
    /// Raw i64; the save does not serialize it.
    pub operand_raw: i64,
    /// Ordered produced-variant handles; a licence may cover several.
    pub equipment: Vec<u64>,
}
typed_codec!(ProductionLicenseRecordRow, 64, |row| (row.owner as u128) << 32 | row.ordinal as u128, |key| Self { owner: (key >> 32) as u32, ordinal: key as u32, giver: Default::default(), lended_cic: Default::default(), required_cic: Default::default(), start_date: Default::default(), parent: Default::default(), operand_raw: Default::default(), equipment: Default::default() },
 { 0 => (giver): U32, 1 => (lended_cic): I32, 2 => (required_cic): I32, 3 => (start_date): I32, 4 => (parent): U64, 5 => (operand_raw): I64, 6 => (equipment): U64List }, |_row| Ok(()));

/// DeploymentConveyors: key native conveyor handle (64 bits), 10 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DeploymentConveyorRow {
    pub handle: u64,
    pub country: u32,
    /// Stored queue position; the engine compacts this vector, so order is state.
    pub ordinal: u32,
    pub template: u64,
    pub amount: i32,
    pub location: u32,
    pub priority: u32,
    /// Raw lexer token; zero is retained and never resolved to text here.
    pub role_token: u32,
    /// Raw engine byte, retained without boolean normalization.
    pub closed: u8,
    /// Literal engine name bytes; an empty name is valid.
    pub name: Vec<u8>,
    /// Raw gated country reference; None when the stored word is not positive, which the save omits under token 0x3ABA.
    pub exile_country: Option<u32>,
}
typed_codec!(DeploymentConveyorRow, 64, |row| row.handle as u128, |key| Self { handle: key as u64, country: Default::default(), ordinal: Default::default(), template: Default::default(), amount: Default::default(), location: Default::default(), priority: Default::default(), role_token: Default::default(), closed: Default::default(), name: Default::default(), exile_country: Default::default() },
 { 0 => (country): U32, 1 => (ordinal): U32, 2 => (template): U64, 3 => (amount): I32, 4 => (location): U32, 5 => (priority): U32, 6 => (role_token): U32, 7 => (closed): U8, 8 => (name): Bytes, 9 => (exile_country): OptionalU32 }, |_row| Ok(()));

/// DeploymentLines: key native line handle (64 bits), 16 payload mask bits. Each line
/// owns exactly one military deployment, whose scalars are merged here.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DeploymentLineRow {
    pub handle: u64,
    pub conveyor: u64,
    pub country: u32,
    pub ordinal: u32,
    /// Native signed order word of the division-name object; zero is a value.
    pub name_order: i32,
    pub amount: i32,
    /// Native i64 fixed5; zero is untrained, not an absent value.
    pub training_raw: i64,
    pub max_training_raw: i64,
    pub max_manpower: i32,
    /// Raw pool byte of the deployment's in-transit equipment pool.
    pub allow_zero_entries: u8,
    /// Native count of the pool's allocation array; the engine cannot write the
    /// saved equipment wrapper while it is zero.
    pub equipment_allocations: i32,
    /// equipment distribution priority; the save omits the default of one.
    pub distributable_priority: i32,
    /// Raw `_bIsSet` byte of the optional halting reason.
    pub halting_reason_present: u8,
    /// Raw halting-reason word; meaningful only when the flag byte is set.
    pub halting_reason: i32,
    /// Literal custom division-name bytes; empty when the stored length is zero, which the save omits under token 0x38e1.
    pub custom_name: Vec<u8>,
    /// Raw ordered flag byte; the save writes token 0x38e2 as `no` only when it is clear.
    pub custom_flag_a: u8,
    /// Raw programmatic flag byte; the save writes token 0x3936 when it is set, saved as override_set_programmatically.
    pub custom_flag_b: u8,
}
typed_codec!(DeploymentLineRow, 64, |row| row.handle as u128, |key| Self { handle: key as u64, conveyor: Default::default(), country: Default::default(), ordinal: Default::default(), name_order: Default::default(), amount: Default::default(), training_raw: Default::default(), max_training_raw: Default::default(), max_manpower: Default::default(), allow_zero_entries: Default::default(), equipment_allocations: Default::default(), distributable_priority: Default::default(), halting_reason_present: Default::default(), halting_reason: Default::default(), custom_name: Default::default(), custom_flag_a: Default::default(), custom_flag_b: Default::default() },
 { 0 => (conveyor): U64, 1 => (country): U32, 2 => (ordinal): U32, 3 => (name_order): I32, 4 => (amount): I32, 5 => (training_raw): I64, 6 => (max_training_raw): I64, 7 => (max_manpower): I32, 8 => (allow_zero_entries): U8, 9 => (equipment_allocations): I32, 10 => (distributable_priority): I32, 11 => (halting_reason_present): U8, 12 => (halting_reason): I32, 13 => (custom_name): Bytes, 14 => (custom_flag_a): U8, 15 => (custom_flag_b): U8 }, |_row| Ok(()));

/// DeploymentManpower: key line:side:ordinal (97 bits), 2 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DeploymentManpowerRow {
    pub line: u64,
    /// Available container zero, needed container one; both always exist.
    pub side: u8,
    pub ordinal: u32,
    /// Raw source-country link exactly as stored; zero is a value.
    pub source_country: u32,
    pub amount: i32,
}
typed_codec!(DeploymentManpowerRow, 97, |row| (row.line as u128) << 33 | (row.side as u128) << 32 | row.ordinal as u128, |key| Self { line: (key >> 33) as u64, side: ((key >> 32) & 1) as u8, ordinal: key as u32, source_country: Default::default(), amount: Default::default() },
 { 0 => (source_country): U32, 1 => (amount): I32 }, |row| if row.side > 1 { Err(invalid("deployment manpower side exceeds one")) } else { Ok(()) });

/// DeploymentEquipment: key line:ordinal (96 bits), 2 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DeploymentEquipmentRow {
    pub line: u64,
    pub ordinal: u32,
    pub variant_handle: u64,
    /// Native i64 fixed5 amount; zero and negative amounts are state.
    pub amount_raw: i64,
}
typed_codec!(DeploymentEquipmentRow, 96, |row| (row.line as u128) << 32 | row.ordinal as u128, |key| Self { line: (key >> 32) as u64, ordinal: key as u32, variant_handle: Default::default(), amount_raw: Default::default() },
 { 0 => (variant_handle): U64, 1 => (amount_raw): I64 }, |_row| Ok(()));

/// LogisticsEquipment: key country:archetype (64 bits), 3 payload mask bits.
///
/// One equipment archetype of a country for which the game computes the
/// Logistics screen's balance (not AI-controlled at the read), from its
/// current daily record. `archetype` is the record index: its position in
/// the equipment database, which `token` names. `balance_raw` (series 16)
/// is the surplus or deficit the Logistics screen's "Stored" column shows,
/// as the day's update computed it; `stock_raw` (series 15) is the stock
/// that update counted. Both fixed5. The appended available operand has
/// the same balance write provenance. `country` is the raw reference. In
/// a complete table, a country without rows was AI-controlled at the read,
/// rather than a country without equipment. The screen recomputes balance
/// at each refresh from live demand, so it can differ later in the day.
/// Its category rows sum a section's archetypes and add incoming lend-lease,
/// which is not recorded.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct LogisticsEquipmentRow {
    pub country: u32,
    pub archetype: u32,
    pub token: u32,
    pub balance_raw: i64,
    pub stock_raw: i64,
    /// Actual available operand of the same native balance write. Older
    /// prefixes omit it; None is a counted missing/refused observation.
    pub available_raw: Recorded<Option<i64>>,
}
typed_codec!(LogisticsEquipmentRow, 64, |row| (row.country as u128) << 32 | row.archetype as u128, |key| Self { country: (key >> 32) as u32, archetype: key as u32, ..Default::default() },
 { 0 => (token): U32, 1 => (balance_raw): I64, 2 => (stock_raw): I64, 3 => (available_raw): OptionalI64 }, |row| {
    if matches!(row.available_raw, Recorded::Value(Some(value)) if value < 0) { return Err(invalid("logistics available operand is negative")) }
    Ok(())
});

/// Immutable applied-source snapshots shared by every design that holds them.
/// Keys describe exact values and attribution; native pointers are not identities.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct VariantBonusSnapshotRow {
    pub id: u64,
    pub source: VariantBonusSource,
}
typed_codec!(VariantBonusSnapshotRow, 64, |row| row.id as u128,
|key| Self { id: key as u64, ..Default::default() }, { 0 => (source): Opaque }, |row| {
    if row.id == 0
        || row.source.stats.iter().any(|entry| entry.index >= engine_reference_values::native_values::equipment_variant::STAT_COUNT as u32) {
        return Err(invalid("invalid applied equipment bonus snapshot"));
    }
    Ok(())
});

/// Process-wide observation used by selected-observer design reveal queries.
/// Unavailable capture is a table refusal, never an assumed enabled switch.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DesignObserverContextRow {
    pub fog: u8,
}
typed_codec!(DesignObserverContextRow, 0, |_row| 0, |_key| Self::default(),
 { 0 => (fog): U8 }, |_row| Ok(()));

/// Key: full organisation reference. `country` is its holding-country link.
/// State is copied verbatim, including signed fixed5 values, the queue's cursor,
/// optional absolute cooldown hour and source-location pair. The unlocked set is
/// the game's completed-trait state; the queue preserves order and repeats.
/// Trait definitions and coordinates come from the recorded content stack.
/// Tasks preserve every native slot and
/// distinguish research template occurrences from production-line references.
/// Variables retain literal keys, exact fixed5 values and the scoped seed;
/// flags retain set dates, signed values and expiry days.
///
/// A failed details read withholds this table alone. IndustrialOrganisations
/// and IndustrialOrganisationHistory retain their independent verdicts.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct IndustrialOrganisationDetailsRow {
    pub organisation: u64,
    pub country: u32,
    pub state: IndustrialOrganisationState,
    pub unlocked: Vec<u32>,
    pub queued: Vec<u32>,
    pub allowed_policies: Vec<u32>,
    pub tasks: Vec<IndustrialOrganisationTask>,
    pub variables: IndustrialOrganisationVariables,
    pub flags: Vec<game_model::script_flag::Row>,
}
typed_codec!(IndustrialOrganisationDetailsRow, 64, |row| row.organisation as u128,
    |key| Self { organisation: key as u64, ..Default::default() }, {
        0 => (country): U32, 1 => (state.funds_raw): I64,
        2 => (state.size): I32, 3 => (state.points): I32,
        4 => (state.research_bonus_raw): I64, 5 => (state.task_capacity): I32,
        6 => (state.research_assign_cost_raw): I64, 7 => (state.production_assign_cost_raw): I64,
        8 => (state.design_team_change_cost): I32, 9 => (state.funds_gain_factor_raw): I64,
        10 => (state.size_cost_factor_raw): I64, 11 => (state.upgrades): U8,
        12 => (state.queue_index): I32, 13 => (state.policy): U32,
        14 => (state.cooldown_hour): OptionalI32, 15 => (state.source_location): Opaque,
        16 => (unlocked): U32List, 17 => (queued): U32List, 18 => (allowed_policies): U32List,
        19 => (tasks): Opaque, 20 => (variables): Opaque, 21 => (flags): Opaque
    }, |_row| Ok(()));

/// Every country's complete policy roster, in native order, including empty
/// rosters. Costs and cooldown days are stored per-country policy values;
/// the selected policy and absolute cooldown belong to each organisation.
/// `cost_modifier_raw` is the country's effective policy-cost modifier in
/// signed fixed5, read in the same visit: the owner's term of every attach
/// quote. `None` means its read failed: unknown, never zero; older formats did not record it.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct IndustrialOrganisationPoliciesRow {
    pub country: u32,
    pub policies: Vec<IndustrialOrganisationPolicy>,
    pub cost_modifier_raw: Recorded<Option<i64>>,
}
typed_codec!(IndustrialOrganisationPoliciesRow, 32, |row| row.country as u128,
    |key| Self { country: key as u32, ..Default::default() },
    { 0 => (policies): Opaque, 1 => (cost_modifier_raw): OptionalI64 }, |_row| Ok(()));

/// Each domain is read only after its writer marks this holder. Unchanged
/// holders are carried under complete writer coverage, including empty holders.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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

/// Kind 0 is a pending request, kind 1 an accepted purchase contract.
/// Ordered manifests, agreed prices and price levels are independent operands.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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
   18 => (variables): Opaque, 19 => (variable_state): U64 }, |row| if row.kind<=1 {Ok(())} else {Err(replay_rows::codec::invalid("market terms kind unsupported"))});

/// Current agreement state. Saved cumulative statistics may saturate and reset
/// on an amendment; actual occurrence amounts remain in MarketEvents.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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
   12 => (flags): U8, 13 => (total_raw): I64, 14 => (lost_total_raw): I64 }, |row| if (row.kind<=30 && row.kind!=19) || row.kind==34 {Ok(())} else {Err(replay_rows::codec::invalid("market event kind unsupported"))});

/// Signed net equipment removal since the preceding consecutive hourly drain,
/// dated by this frame's drain read; negative amounts are native gains (corrections).
/// Historical positive amounts retain their identity and amount when the appended
/// coverage/reason columns are NotRecorded; that absence never certifies coverage.
/// Nonzero (country,cause,design) rows use Complete coverage and an empty
/// unavailable_reason. If the optional hook
/// group failed, one reserved key-zero marker has all identity/amount fields
/// zero, SupplementalUnavailable coverage and its nonempty reason.
/// Consumers must read the marker even when there are no amount rows: net
/// removals are incomplete because deletion, paradrop, merge, exile, volunteer-return,
/// capitulation-escape and scripted division/stockpile causes are unavailable.
/// `cause` is a stable value; engine_events::equipment_destruction owns the executable
/// caller map and evidence. An unrecognized caller records Unknown, counted in
/// collector status, without guessing a known cause or discarding its removals.
/// This table is not the Combat Log and does not cover unhooked convoy-pool or
/// lend-lease losses, scrapping, or equipment captured/transferred between pools.
/// A complete empty hour means zero net removal through the hooked writers, not
/// complete loss coverage for any equipment domain.
/// Variant identity is the same full handle as VariantDefinitions.id.
/// ScriptedStockpileFraction has no fixed domain and is not a land cause.
/// Resolve its variant's equipment_token through recorded LexicalTokens and
/// installed_content_core::equipment::Database from the recorded content stack, then classify
/// that equipment type's effective category with
/// engine_native::cequipment_type::{PLANE_MASK, SHIP_MASK}: air first, naval next,
/// otherwise land. Convoy (bit 0) is naval; train and railway gun are land.
/// Unresolved tokens/content leave the domain unknown.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct EquipmentDestructionRow {
    pub country: u32,
    pub cause: EquipmentDestructionCause,
    pub variant: u64,
    pub amount_raw: i64,
    pub coverage: Recorded<EquipmentDestructionCoverage>,
    pub unavailable_reason: Recorded<String>,
}
typed_codec!(EquipmentDestructionRow, 104,
|row| (row.country as u128) << 72 | (row.cause as u128) << 64 | row.variant as u128,
|key| Self { country: (key >> 72) as u32, cause: EquipmentDestructionCause::try_from((key >> 64) as u8)?, variant: key as u64, ..Default::default() },
{ 0 => (amount_raw): I64, 1 => (coverage): U8, 2 => (unavailable_reason): Bytes }, |row| {
    match (&row.coverage, &row.unavailable_reason) {
        (Recorded::NotRecorded, Recorded::NotRecorded) if row.amount_raw > 0 => Ok(()),
        (Recorded::Value(EquipmentDestructionCoverage::Complete), Recorded::Value(reason)) if row.key() != 0 && row.amount_raw != 0 && reason.is_empty() => Ok(()),
        (Recorded::Value(EquipmentDestructionCoverage::SupplementalUnavailable), Recorded::Value(reason)) if row.key() == 0 && row.amount_raw == 0 && !reason.is_empty() => Ok(()),
        _ => Err(invalid("invalid equipment destruction amount or availability marker")),
    }
});

/// Country production operands copied on the production census's daily slot.
/// Only countries with lines have rows. Definitions are live native values,
/// not constants read from today's vanilla files.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CountryProductionModifiersRow {
    pub country: u32,
    pub power_raw: i64,
    pub base_start: i32,
    pub base_cap: i32,
    pub gain_raw: i64,
    pub balance_raw: i64,
    pub factory_speed: [i64; 4],
    pub retention: [i64; 4],
    pub conversion_raw: i64,
    pub trees: Vec<ProductionModifierTree>,
}
typed_codec!(CountryProductionModifiersRow, 32, |row| row.country as u128,
|key| Self { country: key as u32, ..Default::default() },
{ 0 => (power_raw): I64, 1 => (base_start): I32, 2 => (base_cap): I32,
  3 => (gain_raw): I64, 4 => (balance_raw): I64, 5 => (factory_speed): Opaque,
  6 => (trees): Opaque, 7 => (retention): Opaque, 8 => (conversion_raw): I64 }, |row| {
    if row.trees.len() != engine_reference_values::native_values::production_modifiers::KEYS.len()
        || row.trees.iter().zip(engine_reference_values::native_values::production_modifiers::KEYS).any(|(tree, key)| tree.key != key) {
        return Err(invalid("production modifier keys differ from native request catalogue"));
    }
    for tree in &row.trees {
        if tree.failure.is_empty() { check_tree(&tree.nodes)?; }
        else if !tree.nodes.is_empty() { return Err(invalid("failed production modifier tree retains partial nodes")); }
    }
    Ok(())
});

impl CountryProductionModifiersRow {
    /// The root value of the modifier tree keyed `key`; none when it failed or is absent.
    pub fn modifier(&self, key: u32) -> Option<i64> {
        let tree = self.trees.iter().find(|tree| tree.key == key)?;
        if tree.failure.is_empty() { tree.nodes.first().map(|node| node.value) } else { None }
    }

    /// The powered fraction blended base speed of one military factory (one dockyard when
    /// `naval`), fixed5 a day, under the out-of-power impact modifier `power_modifier`.
    pub fn blended_speed(&self, power_modifier: i64, naval: bool) -> Option<i64> {
        let power = multiply(power_modifier.wrapping_add(100_000), self.power_raw, 100_000)?.max(0);
        let [unpowered, powered] = if naval { [self.factory_speed[2], self.factory_speed[3]] } else { [self.factory_speed[0], self.factory_speed[1]] };
        Some(unpowered.wrapping_add(multiply(power, powered.wrapping_sub(unpowered), 100_000)?))
    }

    /// Fixed5 daily IC one military factory (a dockyard when `naval`) applies before any line's
    /// own terms: the powered base rate with the country's factory or dockyard output modifier,
    /// the powered modifier only at full power, and a positive unpowered modifier reduced by the
    /// unpowered share, as the game's output factor applies them. None when an operand was not
    /// read.
    pub fn factory_output(&self, naval: bool) -> Option<i64> {
        use engine_reference_values::modifier_keys as keys;
        let power_modifier = self.modifier(keys::OUT_OF_POWER_IMPACT_FACTOR)?;
        let base = self.blended_speed(power_modifier, naval)?;
        let (output, powered) = if naval {
            (keys::INDUSTRIAL_CAPACITY_DOCKYARD, keys::INDUSTRIAL_CAPACITY_DOCKYARD_POWERED)
        } else {
            (keys::INDUSTRIAL_CAPACITY_FACTORY, keys::INDUSTRIAL_CAPACITY_FACTORY_POWERED)
        };
        let modifier = self.modifier(output)?;
        let mut total = base.wrapping_add(multiply(base, modifier, 100_000)?);
        if self.power_raw > 99_999 {
            total = total.wrapping_add(multiply(base, self.modifier(powered)?, 100_000)?);
        } else if modifier > 0 {
            let power = multiply(power_modifier.wrapping_add(100_000), self.power_raw, 100_000)?.max(0);
            let adjustment = multiply(100_000_i64.wrapping_sub(power), modifier.wrapping_neg(), 100_000)?;
            total = total.wrapping_add(multiply(base, adjustment, 100_000)?);
        }
        Some(total)
    }
}

/// Line-local cached operands. Country/variant/MIO/count authenticate joins
/// to ProductionLines at the supplying read. Bonuses are the seven runtime
/// production categories after token mapping; dirty remains explicit.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ProductionLineModifiersRow {
    pub line: u64,
    pub country: u32,
    pub variant: u64,
    pub organisation: u64,
    pub kind: u8,
    pub active_factories: i32,
    pub resource_penalty_raw: i64,
    pub organisation_dirty: bool,
    /// Retired at an earlier format; `converting` records the same control.
    pub conversion: RetiredBool,
    pub licensed: bool,
    pub conversion_output: bool,
    pub refit_output: bool,
    pub naval_efficiency: bool,
    pub naval_output: bool,
    pub bonuses: [i64; 7],
    pub factory_efficiencies: Vec<i64>,
    /// The line's conversion control at the read: military production
    /// converts stockpiled equipment into its design while it is set. The
    /// save writes it as `is_converting`. Historical absence is NotRecorded.
    pub converting: Recorded<bool>,
}
typed_codec!(ProductionLineModifiersRow, 64, |row| row.line as u128,
|key| Self { line: key as u64, ..Default::default() },
{ 0 => (country): U32, 1 => (variant): U64, 2 => (organisation): U64, 3 => (kind): U8,
  4 => (active_factories): I32, 5 => (resource_penalty_raw): I64, 6 => (organisation_dirty): Bool,
  7 => (conversion): Bool, 8 => (bonuses): Opaque, 9 => (factory_efficiencies): Opaque,
  10 => (naval_efficiency): Bool, 11 => (naval_output): Bool, 12 => (licensed): Bool,
  13 => (conversion_output): Bool, 14 => (refit_output): Bool, 15 => (converting): Bool },
|row| if row.kind > engine_reference_values::native_values::production_line::KIND_RAILWAY_GUN {
    Err(invalid("production modifier line identity invalid"))
} else { Ok(()) });
