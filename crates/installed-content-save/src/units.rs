use std::{collections::BTreeMap, io};

use installed_content_core::parser::Item;

use super::{
    Decimal, block, boolean, bytes, combat_details, decimal, field, integer, integer_list, invalid,
    null_permitted_handle, object_id, production, required_block, required_bytes, scalar,
    typed_handle,
};

/// One `country_intel` triple of a saved unit, in serialized order.
///
/// The unit writer emits three signed integers per stored viewer entry: the
/// viewer key word, the low 32 bits of the entry's decay accumulator, and the
/// sign-extended level byte. Every entry is written, including nonpositive
/// levels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IntelEntry {
    pub viewer: i32,
    pub timer_low: i32,
    pub level: i32,
}

/// One serialized `railway_gun` in a country's `units` roster.
///
/// A simulation object, not a map counter or a country-view result. The unit
/// writer omits some fields conditionally; options keep that distinction for
/// the recording oracle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RailwayGun {
    pub id: u64,
    /// The unit writer emits `name` only for
    /// nonzero length. The text writer truncates at the first NUL, so the
    /// oracle keeps any difference against the full captured bytes.
    pub name: Option<Vec<u8>>,
    pub roster_owner: Vec<u8>,
    pub logical_owner: Vec<u8>,
    pub expeditionary_owner: Option<Vec<u8>>,
    pub province: Option<u32>,
    pub path: Vec<u32>,
    /// The movement target (`support_attack`) and alternate route (`full_path`).
    pub support_attack: Option<u32>,
    pub full_path: Vec<u32>,
    pub strategic_redeployment: bool,
    /// `exile` is emitted iff its byte is nonzero, as Boolean truth.
    pub exile: bool,
    /// `withdraw=yes` is emitted iff the withdrawal byte is nonzero.
    pub withdrawal: bool,
    pub strength: Decimal,
    /// The gun's own combat reference array, in stored order; the unit writer
    /// never writes the separate unit combat vector.
    pub combats: Vec<u64>,
    /// Serialized per-viewer simulation intel triples, in stored order.
    pub country_intel: Vec<IntelEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Division {
    /// Reference qword: type in the low u32, id in the high u32.
    pub id: u64,
    /// The country's enclosing units roster, independently of logical_country.
    pub roster_owner: Vec<u8>,
    pub logical_owner: Vec<u8>,
    pub expeditionary_owner: Option<Vec<u8>>,
    /// Location is omitted only when its pointer is null.
    pub province: Option<u32>,
    /// The standard template, not fake intel, is emitted iff nonnull.
    pub template: Option<u64>,
    /// The alternate template is written only when nonnull.
    pub alternate_template: Option<u64>,
    /// The separate-counter byte is written only when nonzero; text retains truthiness.
    pub separate_counter: bool,
    /// Saved strength is absolute current hit points in /100000 units.
    pub hit_points: Decimal,
    pub organisation: Decimal,
    /// Ordered viewer entries; the writer omits the field for an empty array.
    pub country_intel: Vec<IntelEntry>,
    pub presentation: DivisionPresentation,
}

/// Fields independently serialized by the unit and army writers.
/// Positive-only fields retain omission, rather than inventing a zero value.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DivisionPresentation {
    /// Stored division XP, independent of exercising status and country XP.
    pub experience: Option<Decimal>,
    pub history: Option<DivisionHistory>,
    pub pending_template: Option<u64>,
    pub officer_experience: Option<Decimal>,
    pub career_counter: Option<i32>,
    /// `fuel_requested`: signed fixed5, omitted exactly at zero.
    pub fuel_requested: Option<Decimal>,
    pub path: Vec<u32>,
    /// The movement target (`support_attack`) and alternate route (`full_path`).
    pub support_attack: Option<u32>,
    pub full_path: Vec<u32>,
    pub previous: Option<u32>,
    pub movement_progress: Option<Decimal>,
    /// `withdraw` is written for a nonzero withdrawal byte.
    pub withdrawal: bool,
    /// `exile` is emitted iff its byte is nonzero, as Boolean truth.
    /// Omission certifies zero; nonzero byte magnitude is not serialized.
    pub exile: bool,
    /// Priority 1 is omitted. The writer emits front_order (0),
    /// player_order (2) or ai_player_order (3); preserve key presence.
    pub movement_priority: Option<i32>,
    pub dig_in: Option<Decimal>,
    pub dig_in_cap: Option<Decimal>,
    /// Maximum supply is written unconditionally.
    pub max_supply: Option<Decimal>,
    /// Current supply ratio is written unconditionally.
    pub army_current_supply_ratio: Option<Decimal>,
    /// Disrupted supply is written only when positive.
    /// Absence proves nonpositive only.
    pub disrupted_supply: Option<Decimal>,
    /// Days out of supply; absent at zero.
    pub out_of_supply_days: Option<i32>,
    /// The division's fuel; absent at zero.
    pub fuel: Option<Decimal>,
    pub strategic_redeployment: bool,
    /// The army's name-group member is written under `division_name`.
    pub division_name: Option<DivisionName>,
    pub acclimatization: Option<Acclimatization>,
}

/// One saved `division_name` block. The writer emits `type`
/// unconditionally and every other key conditionally, so absence is
/// kept distinct from a written value.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DivisionName {
    pub kind: i32,
    /// `name_order`, written only when the order word is nonzero.
    pub order: Option<i32>,
    /// `is_name_ordered`, written as `no` only when the ordered flag is clear.
    pub ordered: Option<bool>,
    /// The custom name when its length is nonzero, outside checksum mode.
    pub custom: Option<Vec<u8>>,
    /// The programmatic flag, written only when set.
    pub programmatic: Option<bool>,
}

/// Each nonzero-weight acclimatization entry is keyed by its category's lexer
/// token in stored order, followed by three engine-owned progress
/// fields, then `max_acclimatization` unless it is 100.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Acclimatization {
    /// (category name, weight) for every nonzero weight, in stored order.
    pub entries: Vec<(Vec<u8>, Decimal)>,
    /// Omitted when the maximum is exactly 100.
    pub maximum: Option<Decimal>,
}

impl Acclimatization {
    /// The maximum the writer omits.
    pub const OMITTED_MAXIMUM_RAW: i64 = 10_000_000;
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DivisionTemplate {
    pub origin_type: Option<Vec<u8>>,
    pub template: u64,
    pub country: Vec<u8>,
    /// Saved bytes, still empty for either omission or an emitted empty value.
    pub name: Vec<u8>,
    /// `name` key presence, independent of its saved byte length. The writer
    /// gates emission on the composition's stored name length and emits its
    /// first-NUL prefix, including `name=""` when a
    /// stored-nonempty string starts with NUL; stored-empty strings are omitted.
    pub name_present: bool,
    /// The composition's saved localisation-key bytes; omission still reads as empty here.
    pub name_key: Vec<u8>,
    /// The same writer independently gates `localization_key` on its stored
    /// length, then emits that string's first-NUL prefix.
    pub name_key_present: bool,
    pub custom_icon: i32,
    pub custom_icon_nato: bool,
    /// `foreign_template_tag`, verbatim.
    pub foreign_template_tag: Option<Vec<u8>>,
    /// Grid kind (regiments/support/regimental support), x, y -> literal unit name.
    pub composition: BTreeMap<(u8, u32, u32), Vec<u8>>,
    /// `division_names_group`: the selected name group's key is written only
    /// when the composition's group is set.
    pub names_group: Option<Vec<u8>>,
    pub settings: DivisionTemplateSettings,
    pub excluded_variants: Vec<u64>,
    pub model_category: Option<u64>,
    pub model_entity: Vec<u8>,
    pub model_entity_present: bool,
}

/// The template writer's saved restriction and equipment settings. Required
/// keys retain their presence, so the oracle cannot turn a missing value into
/// a matching default.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DivisionTemplateSettings {
    pub priority: Option<i32>,
    pub role: Option<Vec<u8>>,
    pub is_army_hq: bool,
    pub locked: bool,
    pub obsolete: bool,
    pub obsolete_change_date: Option<i32>,
    pub force_allow_recruiting: bool,
    pub allow_new_equipment: Option<bool>,
    pub allow_foreign_equipment: Option<bool>,
    pub parent: Option<u64>,
    pub creator_country: Option<Vec<u8>>,
    pub origin_type: Option<Vec<u8>>,
    pub fake_intel: bool,
    pub division_cap: Option<i32>,
    pub equipment_niche: Option<i32>,
    pub forbidden_equipment: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LandCombat {
    pub id: u64,
    pub province: u32,
    pub day: i32,
    pub duration: i32,
    /// Direct side unit references, not the separate front subset.
    pub attackers: Vec<u64>,
    pub defenders: Vec<u64>,
    pub details: combat_details::LandDetails,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fleet {
    pub id: u64,
    pub owner: Vec<u8>,
    pub home_base: Option<u32>,
    /// The fleet name is written except in checksum writer mode.
    pub name: Option<Vec<u8>>,
    pub leader: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskForce {
    pub id: u64,
    pub fleet: u64,
    /// The task-force writer uses the unit writer for `name`. With checksum
    /// mode off it writes the name only when its raw length is nonzero;
    /// text ends at the first NUL.
    /// Omission and saved empty text remain distinct.
    pub name: Option<Vec<u8>>,
    pub roster_owner: Vec<u8>,
    pub logical_owner: Vec<u8>,
    pub expeditionary_owner: Option<Vec<u8>>,
    pub province: Option<u32>,
    pub mission: i32,
    /// Ordered viewer entries; the writer omits the field for an empty array.
    pub country_intel: Vec<IntelEntry>,
    /// The ordinary route is written as `path`, omitted when empty.
    pub path: Vec<u32>,
    /// The movement-target province id, written only for a non-null target, so
    /// `None` is the null pointer. Its saved key is `support_attack`; the
    /// alternate route's is `full_path`. A saved moving division writes
    /// `support_attack=<province>` between `previous` and `last_combat_date`.
    pub target: Option<u32>,
    /// The alternate route, `full_path`, written only when its count is nonzero.
    /// The writer saves no combat list for a task force: its combat vector is
    /// not serialized ([`NavalCombat::attackers`] is the saved membership).
    pub alternate_path: Vec<u32>,
    /// `movement_progress` is written only when nonzero.
    pub movement_progress: Option<Decimal>,
    /// `retreat`/`withdraw` retain Boolean truth only.
    pub retreat: bool,
    pub withdrawal: bool,
    /// `exile` is emitted iff its byte is nonzero, as Boolean truth.
    pub exile: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ship {
    pub ship_name: Option<ShipName>,
    pub manpower: Option<i32>,
    pub max_manpower: Option<i32>,
    /// The serializer emits this wrapper iff its critical array is nonempty,
    /// even when every unnamed entry is omitted from the wrapper.
    pub has_criticals: bool,
    /// Position in the task force serializer's ordered ship list.
    pub roster_ordinal: u32,
    pub id: u64,
    pub task_force: u64,
    pub strength: Decimal,
    pub organisation: Decimal,
    pub experience: Decimal,
    pub sunk_convoys: i32,
    /// The embedded pool is written under `equipment`.
    /// Absence is retained so the oracle can reject a missing required wrapper.
    pub equipment: Option<production::Pool>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArmyOrder {
    pub owner_group: u64,
    pub instance: u32,
    pub country: Vec<u8>,
    pub order_type: i32,
    /// Saved can_execute is a signed byte spelling; preserve all stored bits.
    pub active: u8,
    pub paradrop_target: u32,
    pub path: Vec<u32>,
    pub target_states: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AirWing {
    /// Written only for a signed-positive exile country reference.
    pub government_in_exile_tag: Option<Vec<u8>>,
    pub manpower: Option<i32>,
    pub id: u64,
    pub owner: Vec<u8>,
    /// Ordinary saves emit the raw byte as an unsigned integer unconditionally.
    pub reinforcement_setting: Option<u8>,
    /// Stored text, mandatory even when empty in ordinary saves. Optional
    /// parsing preserves omitted-key evidence for the oracle.
    pub name: Option<Vec<u8>>,
    pub country_override: Vec<u8>,
    pub air_base: u64,
    pub planes: i32,
    pub mission_mask: u32,
    pub mission_region: Option<u32>,
    /// The mission's `active` flag, when written.
    pub mission_active: Option<bool>,
    /// The mission writer emits this key only for a nonzero stored byte.
    pub stop_training_at_max_xp: bool,
    /// Raw fixed5 region-change penalty, written with a mission region.
    pub region_change_penalty: Option<i64>,
    /// Omitted outside the serializer's relocation branch. Total distance
    /// and the destination's direct registration order are not saved.
    pub transfer_progress: Option<i64>,
    pub transferring_to: Option<u32>,
    /// The wing's position among its country's `air_wing_pool` blocks'
    /// `air_wings`, in file order: the pools retain their stored order, as do
    /// each pool's wings.
    pub pool_ordinal: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NavalCombat {
    pub id: u64,
    pub province: u32,
    /// Each side's participant array, written in array order as repeated
    /// `unit` references. Adding a unit appends it here and adds its logical
    /// owner to the side's country set.
    pub attackers: Vec<u64>,
    pub defenders: Vec<u64>,
    pub attacker_positioning: Decimal,
    pub defender_positioning: Decimal,
    pub attacker_anti_air: Decimal,
    pub defender_anti_air: Decimal,
    pub details: combat_details::NavalDetails,
    /// Every `naval_transport` reference in writer order: each slot of the
    /// transfer vector is written, zero and stale ones included.
    pub transfers: Vec<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NavalCombatParticipant {
    pub combat: u64,
    pub side: u8,
    pub battle_slot: u32,
    pub ship: Option<u64>,
    pub convoy: Option<u64>,
    pub owner: Vec<u8>,
    pub state: u32,
    pub details: combat_details::NavalMemberDetails,
}

pub(super) fn parse_division_templates(
    items: &[Item],
) -> io::Result<BTreeMap<u64, DivisionTemplate>> {
    let mut result = BTreeMap::new();
    let Some(value) = field(items, b"division_templates")? else {
        return Ok(result);
    };
    for item in block(value)? {
        if item.key.as_deref() != Some(b"division_template") {
            return Err(invalid("unexpected division template registry entry"));
        }
        let fields = block(&item.value)?;
        let template = typed_handle(
            field(fields, b"id")?.ok_or_else(|| invalid("division template lacks id"))?,
        )?;
        let mut composition = BTreeMap::new();
        for (kind, key) in [b"regiments".as_slice(), b"support", b"regimental_support"]
            .into_iter()
            .enumerate()
        {
            if let Some(value) = field(fields, key)? {
                for unit in block(value)? {
                    let name = unit
                        .key
                        .as_deref()
                        .filter(|s| !s.is_empty())
                        .ok_or_else(|| invalid("template subunit name absent"))?;
                    let position = block(&unit.value)?;
                    let x: u32 = integer(&required_bytes(position, b"x")?)?;
                    let y: u32 = integer(&required_bytes(position, b"y")?)?;
                    if composition
                        .insert((kind as u8, x, y), name.to_vec())
                        .is_some()
                    {
                        return Err(invalid("duplicate division template grid cell"));
                    }
                }
            }
        }
        // Keep save emission presence without changing the existing byte fields
        // or the shared bytes() reader's duplicate/scalar validation.
        let country = required_bytes(fields, b"country")?;
        let name = bytes(fields, b"name")?;
        let name_key = bytes(fields, b"localization_key")?;
        let row = DivisionTemplate {
            origin_type: bytes(fields, b"origin_type")?,
            template,
            country,
            name_present: name.is_some(),
            name: name.unwrap_or_default(),
            name_key_present: name_key.is_some(),
            name_key: name_key.unwrap_or_default(),
            custom_icon: integer(&required_bytes(fields, b"template_counter")?)?,
            custom_icon_nato: boolean(
                field(fields, b"ingame_set_template_counter")?
                    .ok_or_else(|| invalid("template lacks counter override flag"))?,
            )?,
            foreign_template_tag: bytes(fields, b"foreign_template_tag")?,
            composition,
            names_group: bytes(fields, b"division_names_group")?,
            settings: DivisionTemplateSettings {
                role: bytes(fields, b"role")?,
                priority: bytes(fields, b"priority")?
                    .map(|v| integer(&v))
                    .transpose()?,
                is_army_hq: field(fields, b"is_army_hq")?
                    .map(boolean)
                    .transpose()?
                    .unwrap_or(false),
                locked: field(fields, b"is_locked")?
                    .map(boolean)
                    .transpose()?
                    .unwrap_or(false),
                obsolete: field(fields, b"obsolete")?
                    .map(boolean)
                    .transpose()?
                    .unwrap_or(false),
                obsolete_change_date: bytes(fields, b"obsolete_change_date")?
                    .map(|v| super::SavedDate::parse(&v)?.engine_hour())
                    .transpose()?,
                force_allow_recruiting: field(fields, b"force_allow_recruiting")?
                    .map(boolean)
                    .transpose()?
                    .unwrap_or(false),
                allow_new_equipment: field(fields, b"allow_new_equipment")?
                    .map(boolean)
                    .transpose()?,
                allow_foreign_equipment: field(fields, b"allow_foreign_equipment")?
                    .map(boolean)
                    .transpose()?,
                parent: field(fields, b"parent")?.map(typed_handle).transpose()?,
                creator_country: bytes(fields, b"original_tag")?,
                origin_type: bytes(fields, b"origin_type")?,
                fake_intel: field(fields, b"is_fake_intel_division")?
                    .map(boolean)
                    .transpose()?
                    .unwrap_or(false),
                division_cap: bytes(fields, b"division_cap")?
                    .map(|v| integer(&v))
                    .transpose()?,
                equipment_niche: bytes(fields, b"equipment_niche")?
                    .map(|v| integer(&v))
                    .transpose()?,
                forbidden_equipment: match field(fields, b"forbidden_equipment_types")? {
                    None => Vec::new(),
                    Some(value) => block(value)?
                        .iter()
                        .map(|item| {
                            if item.key.as_deref() != Some(b"token") {
                                return Err(invalid("unexpected forbidden template equipment key"));
                            }
                            integer(scalar(&item.value)?)
                        })
                        .collect::<io::Result<_>>()?,
                },
            },
            excluded_variants: match field(fields, b"not_allowed_equipment")? {
                None => Vec::new(),
                Some(value) => block(value)?
                    .iter()
                    .map(|item| {
                        if item.key.as_deref() != Some(b"equipment_variant_index") {
                            return Err(invalid("unexpected template equipment exclusion key"));
                        }
                        typed_handle(&item.value)
                    })
                    .collect::<io::Result<_>>()?,
            },
            model_category: bytes(fields, b"override_model_equipment_category_filter")?
                .map(|v| integer(&v))
                .transpose()?,
            model_entity_present: bytes(fields, b"override_model")?.is_some(),
            model_entity: bytes(fields, b"override_model")?.unwrap_or_default(),
        };
        if result.insert(template, row).is_some() {
            return Err(invalid("duplicate division template handle"));
        }
    }
    Ok(result)
}

#[cfg(test)]
#[path = "units/tests.rs"]
mod tests;

pub(super) fn parse_railway_gun(roster_owner: &[u8], items: &[Item]) -> io::Result<RailwayGun> {
    // The unit and its supply-consumer base can both write the same reference.
    // Equal repeated ids are game output; conflicting ids are malformed.
    let mut id = None;
    for item in items
        .iter()
        .filter(|item| item.key.as_deref() == Some(b"id"))
    {
        let handle = typed_handle(&item.value)?;
        if id.is_some_and(|id| id != handle) {
            return Err(invalid("railway gun has conflicting repeated typed ids"));
        }
        id = Some(handle);
    }
    let id = id.ok_or_else(|| invalid("railway gun lacks its typed id"))?;
    let province = bytes(items, b"location")?
        .map(|value| integer::<u32>(&value))
        .transpose()?;
    if province == Some(0) {
        return Err(invalid(
            "railway gun explicitly saves location zero; null location must be omitted",
        ));
    }
    let combats = match field(items, b"combat")? {
        Some(value) => block(value)?
            .iter()
            .map(|item| {
                if item.key.is_some() {
                    return Err(invalid("railway gun combat list has keyed entry"));
                }
                typed_handle(&item.value)
            })
            .collect::<io::Result<Vec<_>>>()?,
        None => Vec::new(),
    };
    Ok(RailwayGun {
        id,
        name: bytes(items, b"name")?,
        roster_owner: roster_owner.to_vec(),
        logical_owner: required_bytes(items, b"logical_country")?,
        expeditionary_owner: bytes(items, b"expeditionary_owner")?,
        province,
        path: integer_list(items, b"path")?,
        support_attack: bytes(items, b"support_attack")?
            .map(|value| integer(&value))
            .transpose()?,
        full_path: integer_list(items, b"full_path")?,
        strategic_redeployment: field(items, b"strategic_redeployment")?
            .map(boolean)
            .transpose()?
            .unwrap_or(false),
        exile: flag(items, b"exile")?,
        withdrawal: flag(items, b"withdraw")?,
        strength: Decimal::parse(&required_bytes(items, b"strength")?)?,
        combats,
        country_intel: country_intel(items)?,
    })
}

pub(super) fn parse_division(roster_owner: &[u8], items: &[Item]) -> io::Result<Division> {
    // The unit and supply-consumer writers each serialize the same reference object.
    // Repeated equal id blocks are game output; conflicting ones are not.
    let mut id = None;
    for item in items
        .iter()
        .filter(|item| item.key.as_deref() == Some(b"id"))
    {
        let handle = typed_handle(&item.value)?;
        if id.is_some_and(|id| id != handle) {
            return Err(invalid("division has conflicting repeated typed ids"));
        }
        id = Some(handle);
    }
    let id = id.ok_or_else(|| invalid("division lacks its typed id"))?;
    let province = bytes(items, b"location")?
        .map(|value| integer::<u32>(&value))
        .transpose()?;
    if province == Some(0) {
        return Err(invalid(
            "division explicitly saves location zero; null location must be omitted",
        ));
    }
    let disrupted_supply = decimal(items, b"disrupted_supply")?;
    if disrupted_supply
        .as_ref()
        .is_some_and(|value| value.raw <= 0)
    {
        return Err(invalid(
            "division explicitly saves nonpositive disrupted_supply; the writer must omit it",
        ));
    }
    Ok(Division {
        id,
        roster_owner: roster_owner.to_vec(),
        logical_owner: required_bytes(items, b"logical_country")?,
        expeditionary_owner: bytes(items, b"expeditionary_owner")?,
        province,
        template: field(items, b"division_template_id")?
            .map(typed_handle)
            .transpose()?,
        alternate_template: field(items, b"fake_intel_division_template_id")?
            .map(typed_handle)
            .transpose()?,
        separate_counter: field(items, b"leader")?
            .map(boolean)
            .transpose()?
            .unwrap_or(false),
        hit_points: Decimal::parse(&required_bytes(items, b"strength")?)?,
        organisation: Decimal::parse(&required_bytes(items, b"organisation")?)?,
        country_intel: country_intel(items)?,
        presentation: DivisionPresentation {
            experience: decimal(items, b"experience")?,
            pending_template: field(items, b"old_division_template_id")?
                .map(typed_handle)
                .transpose()?,
            history: field(items, b"army_history")?
                .map(|value| division_history(block(value)?))
                .transpose()?,
            officer_experience: field(items, b"held_officer")?
                .map(|value| decimal(block(value)?, b"experience"))
                .transpose()?
                .flatten(),
            career_counter: bytes(items, b"killed")?
                .map(|value| integer(&value))
                .transpose()?,
            fuel_requested: decimal(items, b"fuel_requested")?,
            path: integer_list(items, b"path")?,
            support_attack: bytes(items, b"support_attack")?
                .map(|value| integer(&value))
                .transpose()?,
            full_path: integer_list(items, b"full_path")?,
            previous: bytes(items, b"previous")?
                .map(|b| integer(&b))
                .transpose()?,
            movement_progress: decimal(items, b"movement_progress")?,
            withdrawal: flag(items, b"withdraw")?,
            exile: flag(items, b"exile")?,
            movement_priority: movement_priority(items)?,
            dig_in: decimal(items, b"dig_in")?,
            dig_in_cap: decimal(items, b"dig_in_cap")?,
            max_supply: decimal(items, b"max_supply")?,
            army_current_supply_ratio: decimal(items, b"army_current_supply_ratio")?,
            disrupted_supply,
            out_of_supply_days: bytes(items, b"out_of_supply_days")?
                .map(|value| integer(&value))
                .transpose()?,
            fuel: decimal(items, b"fuel")?,
            strategic_redeployment: field(items, b"strategic_redeployment")?
                .map(boolean)
                .transpose()?
                .unwrap_or(false),
            division_name: field(items, b"division_name")?
                .map(|value| division_name(block(value)?))
                .transpose()?,
            acclimatization: acclimatization(items)?,
        },
    })
}

pub(super) fn division_name(items: &[Item]) -> io::Result<DivisionName> {
    let flag = |key: &[u8]| field(items, key)?.map(boolean).transpose();
    Ok(DivisionName {
        kind: integer(&required_bytes(items, b"type")?)?,
        order: bytes(items, b"name_order")?
            .map(|value| integer(&value))
            .transpose()?,
        ordered: flag(b"is_name_ordered")?,
        custom: bytes(items, b"override")?,
        programmatic: flag(b"override_set_programmatically")?,
    })
}

fn flag(items: &[Item], key: &[u8]) -> io::Result<bool> {
    Ok(field(items, key)?
        .map(boolean)
        .transpose()?
        .unwrap_or(false))
}

/// The default priority is never written. Unknown stored words take the
/// front_order branch, so its spelling cannot certify their bits.
fn movement_priority(items: &[Item]) -> io::Result<Option<i32>> {
    bytes(items, b"move_priority")?
        .map(|value| match value.as_slice() {
            b"front_order" => Ok(0),
            b"player_order" => Ok(2),
            b"ai_player_order" => Ok(3),
            _ => Err(invalid("unsupported saved move_priority")),
        })
        .transpose()
}

fn acclimatization(items: &[Item]) -> io::Result<Option<Acclimatization>> {
    let Some(value) = field(items, b"acclimatization")? else {
        return Ok(None);
    };
    let mut result = Acclimatization::default();
    for item in block(value)? {
        let key = item
            .key
            .as_deref()
            .filter(|key| !key.is_empty())
            .ok_or_else(|| invalid("acclimatization entry lacks a key"))?;
        match key {
            // The writer's own progress fields.
            b"actively_gaining" | b"actively_gaining_speed" | b"other_loss" => {}
            b"max_acclimatization" => {
                if result
                    .maximum
                    .replace(Decimal::parse(scalar(&item.value)?)?)
                    .is_some()
                {
                    return Err(invalid("duplicate max_acclimatization"));
                }
            }
            category => result
                .entries
                .push((category.to_vec(), Decimal::parse(scalar(&item.value)?)?)),
        }
    }
    Ok(Some(result))
}

/// The writer emits one anonymous flat list of three signed integers per entry.
fn country_intel(items: &[Item]) -> io::Result<Vec<IntelEntry>> {
    let Some(value) = field(items, b"country_intel")? else {
        return Ok(Vec::new());
    };
    let entries = block(value)?;
    if entries.is_empty() || entries.len() % 3 != 0 {
        return Err(invalid(
            "saved country_intel is empty or not a whole number of viewer triples",
        ));
    }
    let mut words = Vec::with_capacity(entries.len());
    for entry in entries {
        if entry.key.is_some() {
            return Err(invalid("saved country_intel must be an anonymous list"));
        }
        words.push(integer::<i32>(scalar(&entry.value)?)?);
    }
    Ok(words
        .chunks_exact(3)
        .map(|triple| IntelEntry {
            viewer: triple[0],
            timer_low: triple[1],
            level: triple[2],
        })
        .collect())
}

pub(super) fn parse_land_combat(items: &[Item], border_war: bool) -> io::Result<LandCombat> {
    fn participants(items: &[Item], side: &[u8]) -> io::Result<Vec<u64>> {
        required_block(items, side)?
            .iter()
            .filter(|item| item.key.as_deref() == Some(b"unit"))
            .map(|item| typed_handle(&item.value))
            .collect()
    }
    Ok(LandCombat {
        id: typed_handle(
            field(items, b"id")?.ok_or_else(|| invalid("land combat lacks typed id"))?,
        )?,
        province: integer::<i32>(&required_bytes(items, b"location")?)? as u32,
        day: integer(&required_bytes(items, b"day")?)?,
        duration: integer(&required_bytes(items, b"duration")?)?,
        attackers: participants(items, b"attacker")?,
        defenders: participants(items, b"defender")?,
        details: combat_details::land(items, border_war)?,
    })
}

pub(super) fn parse_fleet(owner: &[u8], items: &[Item]) -> io::Result<Fleet> {
    Ok(Fleet {
        id: object_id(items)?,
        owner: owner.to_vec(),
        name: bytes(items, b"name")?,
        leader: field(items, b"leader")?.map(typed_handle).transpose()?,
        home_base: field(items, b"home_base")?
            .map(|value| integer(scalar(value)?))
            .transpose()?,
    })
}

pub(super) fn parse_task_force(owner: &[u8], fleet: u64, items: &[Item]) -> io::Result<TaskForce> {
    Ok(TaskForce {
        id: object_id(items)?,
        fleet,
        name: bytes(items, b"name")?,
        roster_owner: owner.to_vec(),
        logical_owner: required_bytes(items, b"logical_country")?,
        expeditionary_owner: bytes(items, b"expeditionary_owner")?,
        province: field(items, b"location")?
            .map(|value| integer(scalar(value)?))
            .transpose()?,
        mission: integer(&required_bytes(
            required_block(items, b"mission")?,
            b"mission",
        )?)?,
        country_intel: country_intel(items)?,
        path: integer_list(items, b"path")?,
        target: field(items, b"support_attack")?
            .map(|value| integer::<i32>(scalar(value)?).map(|id| id as u32))
            .transpose()?,
        alternate_path: integer_list(items, b"full_path")?,
        movement_progress: decimal(items, b"movement_progress")?,
        retreat: flag(items, b"retreat")?,
        withdrawal: flag(items, b"withdraw")?,
        exile: flag(items, b"exile")?,
    })
}

/// Saved ship-name wrapper. Optional operands retain omission independently
/// of explicit empty, zero and false spellings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShipName {
    pub kind: i32,
    pub name_order: Option<i32>,
    pub is_name_ordered: Option<bool>,
    pub override_name: Option<Vec<u8>>,
    pub override_set_programmatically: Option<bool>,
    pub equipment: Option<u64>,
}
fn ship_name(items: &[Item]) -> io::Result<ShipName> {
    Ok(ShipName {
        kind: integer(&required_bytes(items, b"type")?)?,
        name_order: field(items, b"name_order")?
            .map(|value| integer(scalar(value)?))
            .transpose()?,
        is_name_ordered: field(items, b"is_name_ordered")?.map(boolean).transpose()?,
        override_name: bytes(items, b"override")?,
        override_set_programmatically: field(items, b"override_set_programmatically")?
            .map(boolean)
            .transpose()?,
        equipment: field(items, b"equipment")?.map(typed_handle).transpose()?,
    })
}

pub(super) fn parse_ship(task_force: u64, items: &[Item]) -> io::Result<Ship> {
    Ok(Ship {
        ship_name: field(items, b"ship_name")?
            .map(block)
            .transpose()?
            .map(ship_name)
            .transpose()?,
        manpower: field(items, b"manpower")?
            .map(|value| integer(scalar(value)?))
            .transpose()?,
        max_manpower: field(items, b"max_manpower")?
            .map(|value| integer(scalar(value)?))
            .transpose()?,
        has_criticals: field(items, b"critical_damage")?
            .map(block)
            .transpose()?
            .is_some(),
        roster_ordinal: 0,
        id: object_id(items)?,
        task_force,
        strength: decimal(items, b"strength")?.ok_or_else(|| invalid("ship lacks strength"))?,
        organisation: decimal(items, b"organisation")?
            .ok_or_else(|| invalid("ship lacks organisation"))?,
        experience: decimal(items, b"experience")?.unwrap_or(Decimal::parse(b"0")?),
        sunk_convoys: field(items, b"sunk_convoys")?
            .map(|value| integer(scalar(value)?))
            .transpose()?
            .unwrap_or(0),
        equipment: field(items, b"equipment")?
            .map(block)
            .transpose()?
            .map(production::parse_pool)
            .transpose()?,
    })
}

fn parse_army_order(country: &[u8], owner_group: u64, items: &[Item]) -> io::Result<ArmyOrder> {
    Ok(ArmyOrder {
        owner_group,
        country: country.to_vec(),
        instance: integer(&required_bytes(items, b"instance_id")?)?,
        order_type: integer(&required_bytes(items, b"type")?)?,
        active: field(items, b"can_execute")?
            .map(|value| integer::<i8>(scalar(value)?))
            .transpose()?
            .unwrap_or(0) as u8,
        paradrop_target: field(items, b"invasion_source")?
            .map(|value| integer::<i32>(scalar(value)?))
            .transpose()?
            .unwrap_or(0) as u32,
        path: integer_list(items, b"path")?,
        target_states: integer_list(items, b"states")?,
    })
}

pub(super) fn parse_orders(
    country: &[u8],
    items: &[Item],
    orders: &mut BTreeMap<(u64, u32), ArmyOrder>,
) -> io::Result<()> {
    let Some(theatres) = field(items, b"theatres")? else {
        return Ok(());
    };
    for theatre in block(theatres)?
        .iter()
        .filter(|item| item.key.as_deref() == Some(b"theatre"))
    {
        for group in block(&theatre.value)?.iter().filter(|item| {
            matches!(
                item.key.as_deref(),
                Some(b"orders_group" | b"field_marshal_group")
            )
        }) {
            let group = block(&group.value)?;
            let id = object_id(group)?;
            for item in group.iter().filter(|item| {
                matches!(
                    item.key.as_deref(),
                    Some(b"order_instance" | b"fallback" | b"virtual_fallback")
                )
            }) {
                let order = parse_army_order(country, id, block(&item.value)?)?;
                if orders.insert((id, order.instance), order).is_some() {
                    return Err(invalid("duplicate scoped army order instance in save"));
                }
            }
        }
    }
    Ok(())
}

pub(super) fn parse_air_wings(items: &[Item]) -> io::Result<BTreeMap<u64, AirWing>> {
    let mut wings = BTreeMap::new();
    let Some(air) = field(items, b"strategic_air")? else {
        return Ok(wings);
    };
    for country in block(air)? {
        let owner = country
            .key
            .as_deref()
            .filter(|key| !key.is_empty())
            .ok_or_else(|| invalid("strategic air country lacks tag"))?;
        let mut pool_ordinal = 0_u32;
        for pool in block(&country.value)?
            .iter()
            .filter(|item| item.key.as_deref() == Some(b"air_wing_pool"))
        {
            let pool = block(&pool.value)?;
            for wing in pool
                .iter()
                .filter(|item| item.key.as_deref() == Some(b"air_wings"))
            {
                let fields = block(&wing.value)?;
                let mission = required_block(fields, b"mission")?;
                let wing = AirWing {
                    government_in_exile_tag: bytes(fields, b"government_in_exile_tag")?,
                    manpower: field(fields, b"manpower")?
                        .map(|value| integer(scalar(value)?))
                        .transpose()?,
                    id: object_id(fields)?,
                    owner: owner.to_vec(),
                    reinforcement_setting: field(fields, b"reinforcement_setting")?
                        .map(|value| integer(scalar(value)?))
                        .transpose()?,
                    name: field(fields, b"name")?
                        .map(|value| scalar(value).map(Vec::from))
                        .transpose()?,
                    country_override: required_bytes(fields, b"tag")?,
                    air_base: typed_handle(
                        field(pool, b"air_base")?
                            .ok_or_else(|| invalid("populated air pool lacks air_base"))?,
                    )?,
                    planes: integer(&required_bytes(fields, b"count")?)?,
                    mission_mask: integer::<i32>(&required_bytes(mission, b"type")?)? as u32,
                    mission_region: field(mission, b"strategic_region")?
                        .map(|value| integer(scalar(value)?))
                        .transpose()?,
                    mission_active: field(mission, b"active")?.map(boolean).transpose()?,
                    stop_training_at_max_xp: field(mission, b"stop_training_at_max_xp")?
                        .map(boolean)
                        .transpose()?
                        .unwrap_or(false),
                    region_change_penalty: decimal(mission, b"region_change_penalty")?
                        .map(|penalty| penalty.raw),
                    transfer_progress: decimal(fields, b"transfer_progress")?
                        .map(|value| value.raw),
                    transferring_to: field(fields, b"transferring_to")?
                        .map(|value| integer(scalar(value)?))
                        .transpose()?,
                    pool_ordinal,
                };
                pool_ordinal = pool_ordinal
                    .checked_add(1)
                    .ok_or_else(|| invalid("air wing pool ordinal exceeds u32"))?;
                if wings.insert(wing.id, wing).is_some() {
                    return Err(invalid("duplicate full air wing handle in save"));
                }
            }
        }
    }
    Ok(wings)
}

pub(super) fn parse_naval_combat(
    items: &[Item],
    participants: &mut BTreeMap<(u64, u8, u32), NavalCombatParticipant>,
) -> io::Result<NavalCombat> {
    let id = object_id(items)?;
    let attacker = required_block(items, b"attacker")?;
    let defender = required_block(items, b"defender")?;
    for (side, fields) in [(0, attacker), (1, defender)] {
        for group in fields
            .iter()
            .filter(|item| item.key.as_deref() == Some(b"group"))
        {
            for item in block(&group.value)?
                .iter()
                .filter(|item| item.key.as_deref() == Some(b"member"))
            {
                let fields = block(&item.value)?;
                let row = combat_details::naval_participant(fields, id, side)?;
                if participants
                    .insert((id, side, row.battle_slot), row)
                    .is_some()
                {
                    return Err(invalid("duplicate combat side battle slot in save"));
                }
            }
        }
    }
    let units = |side: &[Item]| {
        side.iter()
            .filter(|item| item.key.as_deref() == Some(b"unit"))
            .map(|item| typed_handle(&item.value))
            .collect::<io::Result<Vec<_>>>()
    };
    Ok(NavalCombat {
        id,
        province: integer::<i32>(&required_bytes(items, b"location")?)? as u32,
        attackers: units(attacker)?,
        defenders: units(defender)?,
        attacker_positioning: Decimal::parse(&required_bytes(attacker, b"positioning")?)?,
        defender_positioning: Decimal::parse(&required_bytes(defender, b"positioning")?)?,
        attacker_anti_air: Decimal::parse(&required_bytes(attacker, b"anti_air")?)?,
        defender_anti_air: Decimal::parse(&required_bytes(defender, b"anti_air")?)?,
        details: combat_details::naval(items)?,
        transfers: items
            .iter()
            .filter(|item| item.key.as_deref() == Some(b"naval_transport"))
            .map(|item| null_permitted_handle(&item.value))
            .collect::<io::Result<_>>()?,
    })
}

/// The saved division's ordered career and awarded-medal lists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DivisionHistory {
    pub amount: Option<i32>,
    pub career: Vec<DivisionHistoryEntry>,
    pub awards: Vec<DivisionHistoryEntry>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DivisionHistoryEntry {
    pub name: Option<Vec<u8>>,
    pub custom_name: Option<Vec<u8>>,
    pub kind: i32,
    pub target_country: Vec<u8>,
    pub hour: i32,
    pub medal: Option<Vec<u8>>,
    pub medal_count: bool,
    pub inherit: bool,
    pub multiplier: Option<Decimal>,
    pub orders: Option<i32>,
    pub provinces: Vec<u32>,
    pub sunk_ship: Option<super::naval_history::SunkShip>,
}
fn division_history(items: &[Item]) -> io::Result<DivisionHistory> {
    fn entries(items: &[Item], key: &[u8]) -> io::Result<Vec<DivisionHistoryEntry>> {
        let Some(value) = field(items, key)? else {
            return Ok(Vec::new());
        };
        block(value)?
            .iter()
            .filter(|item| item.key.as_deref() == Some(b"history_queue"))
            .map(|item| {
                let fields = block(&item.value)?;
                Ok(DivisionHistoryEntry {
                    name: bytes(fields, b"army_names")?,
                    custom_name: bytes(fields, b"custom_lockey")?,
                    kind: integer(&required_bytes(fields, b"unique")?)?,
                    target_country: required_bytes(fields, b"target_country")?,
                    hour: super::SavedDate::parse(&required_bytes(fields, b"date")?)?
                        .engine_hour()?,
                    medal: bytes(fields, b"unit_medals")?,
                    medal_count: boolean(
                        field(fields, b"medal_count")?
                            .ok_or_else(|| invalid("save lacks medal_count"))?,
                    )?,
                    inherit: boolean(
                        field(fields, b"inherit")?.ok_or_else(|| invalid("save lacks inherit"))?,
                    )?,
                    multiplier: decimal(fields, b"multiplier")?,
                    orders: bytes(fields, b"orders")?
                        .map(|value| integer(&value))
                        .transpose()?,
                    provinces: integer_list(fields, b"location")?,
                    sunk_ship: field(fields, b"sunk_ship")?
                        .map(|value| super::naval_history::sunk_ship(block(value)?))
                        .transpose()?,
                })
            })
            .collect()
    }
    let medals = field(items, b"unit_medals")?
        .map(block)
        .transpose()?
        .unwrap_or(&[]);
    Ok(DivisionHistory {
        amount: bytes(medals, b"amount")?
            .map(|value| integer(&value))
            .transpose()?,
        career: entries(items, b"army_history")?,
        awards: entries(medals, b"history")?,
    })
}

#[cfg(test)]
mod ship_name_tests {
    use super::*;
    fn ship(text: &str) -> io::Result<Ship> {
        let document = installed_content_core::parser::parse(
            format!("id={{id=4 type=51}} strength=0 organisation=0 {text}").as_bytes(),
        )?;
        parse_ship(9, &document.items)
    }
    #[test]
    fn ship_names_keep_missing_empty_signed_flags_and_full_equipment() {
        assert!(ship("").unwrap().ship_name.is_none());
        let missing = ship("ship_name={type=1}").unwrap().ship_name.unwrap();
        assert_eq!(missing.override_name, None);
        let name = ship(r#"ship_name={type=1 name_order=-7 is_name_ordered=no override="" override_set_programmatically=yes equipment={id=4294967295 type=70}}"#)
            .unwrap().ship_name.unwrap();
        assert_eq!(name.override_name, Some(Vec::new()));
        assert_eq!(name.name_order, Some(-7));
        assert_eq!(name.is_name_ordered, Some(false));
        assert_eq!(name.override_set_programmatically, Some(true));
        assert_eq!(name.equipment, Some((u64::from(u32::MAX) << 32) | 70));
        let name = ship(
            "ship_name={type=-1 name_order=0 is_name_ordered=yes override_set_programmatically=no}",
        )
        .unwrap()
        .ship_name
        .unwrap();
        assert_eq!(name.kind, -1);
        assert_eq!(name.name_order, Some(0));
        assert_eq!(name.is_name_ordered, Some(true));
        assert_eq!(name.override_set_programmatically, Some(false));
    }
    #[test]
    fn ship_names_reject_missing_type_invalid_and_duplicate_fields() {
        for text in [
            "ship_name={}",
            "ship_name={type=x}",
            "ship_name={type=1 is_name_ordered=1}",
            "ship_name={type=1 equipment={id=0 type=0}}",
            "ship_name={type=1} ship_name={type=1}",
        ] {
            assert!(ship(text).is_err(), "{text}");
        }
        for field in [
            "type=1",
            "name_order=-1",
            "is_name_ordered=no",
            "override=\"\"",
            "override_set_programmatically=yes",
            "equipment={id=1 type=70}",
        ] {
            let prefix = if field.starts_with("type=") {
                ""
            } else {
                "type=1"
            };
            let text = format!("ship_name={{{prefix} {field} {field}}}");
            assert!(ship(&text).is_err(), "{text}");
        }
    }
}
